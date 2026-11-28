// The transport (docs/07 R2.5, R3): the processor's path for a scene of
// OpenPBR surfaces (openpbr.wgsl), glass among them, under the environment
// and the analytic lamps (lamps.wgsl), standing on a shadow catcher, draw
// for draw - camera ray, next-event estimation of the sky by its facing
// tables (the whole sky through glass) with power-heuristic MIS, of one
// lamp chosen as the processor chooses it and of one glowing triangle chosen
// down the processor's tree of them (emitters.wgsl), weighed against the
// bounces that hit a glow, shadow rays through clear glass,
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
//#include features
//#include sampler
//#include environment
//#include openpbr
//#include lamps
//#include emitters
//#include program
//#include flakes
//#include dispersion
//#include spectral
//#include media

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

// clamp_light for a path carrying wavelengths (clamp_share): the share
// that brings the red, green and blue they register as down to the ceiling.
fn clamp_light4(c: vec4<f32>, ceiling: f32) -> vec4<f32> {
    if (!(ceiling > 0.0)) {
        return c;
    }
    let rgb = spectral_rgb(c);
    let sum = abs(rgb.x) + abs(rgb.y) + abs(rgb.z);
    if (sum > ceiling) {
        return c * (ceiling / sum);
    }
    return c;
}

// Whether the sample being traced carries wavelengths (frame.mode's
// MODE_SPECTRAL), set at its start.
var<private> spectral_on: bool;

// light_ceiling, raised by what Russian roulette has scaled the path up by
// (#207): the light judged as it would be without the roulette, whose
// survivors carry the share of the paths it ended. A ceiling at or below
// nought stays none.
fn light_ceiling(specular_chain: bool, found_at: u32, roulette: f32) -> f32 {
    if (found_at == 0u || specular_chain) {
        return frame.clamp_direct * roulette;
    }
    return frame.clamp_indirect * roulette;
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
            case LOBE_COAT: { if (!ggx_is_smooth(b.coat.distribution)) { return false; } }
            case LOBE_FLAKE: { if (!ggx_is_smooth(b.flake.distribution)) { return false; } }
            case LOBE_METAL: { if (!ggx_is_smooth(b.metal.distribution)) { return false; } }
            case LOBE_SPECULAR: { if (!ggx_is_smooth(b.specular.distribution)) { return false; } }
            case LOBE_TRANSMISSION: { if (!ggx_is_smooth(b.transmission.distribution)) { return false; } }
            default: { return false; }
        }
    }
    return true;
}

struct PathOut {
    radiance: vec3<f32>,
    background: vec3<f32>,
    coverage: f32,
    // The one wavelength the camera's path carried from a dispersive
    // surface on (PathResult::wavelength), nought for none.
    wavelength: f32,
    // Carrying wavelengths, how luminous the film finds the camera path's
    // light (PathResult::hue).
    hue: f32,
};

fn is_finite(x: f32) -> bool {
    return (bitcast<u32>(x) & 0x7f800000u) != 0x7f800000u;
}

// PathColour::finish: an RGB path's radiance made safe (sanitized); a
// spectral one's only its non-finite values dropped, a negative channel
// kept, as one path's estimate of a saturated colour can be.
fn finished(c: vec3<f32>) -> vec3<f32> {
    if (FEATURE_SPECTRAL && spectral_on) {
        return select(vec3<f32>(0.0), c, abs(c) <= vec3<f32>(3.40282347e38));
    }
    return sanitized(c);
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

// A medium a path is inside (ic:338's Medium, without Volume Maps): the
// solid it is, its priority, its index, and the material whose volume
// absorbs along the path (its GLASS_VOLUME) or scatters (GLASS_SCATTERS);
// or, entered through a subsurface lobe and left through one, a walk under
// the material's surface (`doorway`, Medium::walk) and the albedo of its
// medium as the shaded material had it.
struct Medium {
    solid: u32,
    priority: f32,
    ior: f32,
    material: u32,
    doorway: bool,
    albedo: vec3<f32>,
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
    media_cross_walk(m, solid, priority, ior, material, false, vec3<f32>(0.0));
}

// MediumStack::cross_with, or with `doorway` cross_walk: crossing a surface
// of `solid` through its subsurface lobe leaves the medium of `solid` the
// path is in, or starts a walk under it (Medium::walk, of the shaded
// material: its priority, its index and its walk's albedo); a fifth medium
// is not recorded.
fn media_cross_walk(m: ptr<function, Media>, solid: u32, priority: f32, ior: f32, material: u32, doorway: bool, albedo: vec3<f32>) {
    let at = media_position(*m, solid);
    if (at != NO_MEDIUM) {
        for (var j = at; j + 1u < (*m).len; j++) {
            (*m).media[j] = (*m).media[j + 1u];
        }
        (*m).len -= 1u;
    } else if ((*m).len < MAX_MEDIA) {
        (*m).media[(*m).len] = Medium(solid, priority, ior, material, doorway, albedo);
        (*m).len += 1u;
    }
}

// MediumStack::walking_in: whether the path is on a walk under `solid`'s
// surface, the medium present.
fn media_walking_in(m: Media, solid: u32) -> bool {
    let top = media_top(m, NO_MEDIUM);
    return top != NO_MEDIUM && m.media[top].doorway && m.media[top].solid == solid;
}

// ---- Media that scatter (ic:338-512, 967-1256, 4112-4273) -----------------

// How many steps a random walk may take before it is taken as absorbed
// (ic:306).
const MAX_WALK_STEPS: u32 = 16384u;

// Whether a medium scatters as well as absorbs (Medium::scattering): a
// volume that does (GLASS_SCATTERS, Material::volume_scattering).
fn medium_scatters(m: Medium) -> bool {
    return (FEATURE_SUBSURFACE && m.doorway) || (FEATURE_SCATTERING && (scene_materials[m.material].glass & GLASS_SCATTERS) != 0u);
}

// The medium a walk crosses (SubsurfaceMedium): its extinction per metre,
// its single-scattering albedo and its phase function's anisotropy.
struct Scattering {
    sigma: vec3<f32>,
    albedo: vec3<f32>,
    g: f32,
};

fn medium_scattering(m: Medium) -> Scattering {
    let r = scene_materials[m.material];
    if (FEATURE_SUBSURFACE && m.doorway) {
        return Scattering(r.subsurface_extinction.xyz, m.albedo, r.subsurface_extinction.w);
    }
    return Scattering(r.volume_extinction.xyz, r.volume_albedo.xyz, r.volume_extinction.w);
}

// One component of a cross product as the processor works it out in f64 and
// rounds to f32 (Vec3::cross): a d - b e, its two products exact, rounded
// once; a nought signed as f64 signs it.
fn exact_cross_term(a: f32, d: f32, b: f32, e: f32) -> f32 {
    let p1 = cy_exact(a * d);
    let e1 = fma(a, d, -p1);
    let p2 = cy_exact(b * e);
    let e2 = fma(b, e, -p2);
    if (e1 == 0.0 && e2 == 0.0) {
        return p1 - p2;
    }
    let s = tex_two_sum(p1, -p2);
    return s.x + (s.y + (e1 - e2));
}

// The geometric normal of `hit` as the processor's resolve has it
// (embree.rs: Mesh::geometric_normal_scaled through Transform::normal, then
// try_normalize), turned to face `direction`'s origin (facing_forward): down
// to the sign of a nought, which turns a frame about the normal
// (Onb::from_normal) the other way round. What a walk is guided about.
fn exact_geometric(hit: RayIntersection, direction: vec3<f32>) -> vec3<f32> {
    let inst = scene_instances[hit.instance_custom_data];
    let mesh = scene_meshes[inst.mesh];
    let first = mesh.first_index + 3u * hit.primitive_index;
    let c0 = scene_position(mesh.base_vertex + scene_indices[first]);
    let a = scene_position(mesh.base_vertex + scene_indices[first + 1u]) - c0;
    let b = scene_position(mesh.base_vertex + scene_indices[first + 2u]) - c0;
    let n = vec3<f32>(exact_cross_term(a.y, b.z, a.z, b.y), exact_cross_term(a.z, b.x, a.x, b.z), exact_cross_term(a.x, b.y, a.y, b.x));
    let m = inst.world_to_object;
    let w = vec3<f32>(
        (cy_exact(m[0].x * n.x) + cy_exact(m[1].x * n.y)) + cy_exact(m[2].x * n.z),
        (cy_exact(m[0].y * n.x) + cy_exact(m[1].y * n.y)) + cy_exact(m[2].y * n.z),
        (cy_exact(m[0].z * n.x) + cy_exact(m[1].z * n.y)) + cy_exact(m[2].z * n.z),
    );
    let len2 = (cy_exact(w.x * w.x) + cy_exact(w.y * w.y)) + cy_exact(w.z * w.z);
    var g = vec3<f32>(0.0, 1.0, 0.0);
    if (len2 > 0.0 && len2 <= 3.4e38) {
        let inv = tex_div(1.0, cy_sqrt(len2));
        g = vec3<f32>(cy_exact(w.x * inv), cy_exact(w.y * inv), cy_exact(w.z * inv));
    }
    let facing = (cy_exact(direction.x * g.x) + cy_exact(direction.y * g.y)) + cy_exact(direction.z * g.z);
    return select(g, -g, facing > 0.0);
}

// The surface a random walk leaves by (walk_exit, ic:316): the subsurface
// lobe alone, white, nothing reflecting in front of it - `m` with every
// other lobe's weight nought.
fn walk_exit_material(m: GpuMaterial) -> GpuMaterial {
    var e = m;
    e.base_weight = 1.0;
    e.base_metalness = 0.0;
    e.specular_weight = 0.0;
    e.transmission_color.w = 0.0;
    e.coat_weight = 0.0;
    e.fuzz_weight = 0.0;
    e.geometry_occlusion = 1.0;
    e.base_color = vec4<f32>(0.18, 0.18, 0.18, 0.0);
    e.subsurface_color = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    e.glass = 0u;
    // No flake, no film, no dispersion: walk_exit is a default material
    // but for what it sets.
    e.flake_color.w = 0.0;
    e.flake.w = 0.0;
    e.film = vec4<f32>(0.0);
    return e;
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

// Material::emitted_over: what `distance` of the volume gives off, less
// what it absorbs of that on the way out, channel by channel - the glow
// times the distance where it absorbs next to nothing, else
// glow (1 - e^(-s d)) / s for an absorption of s a metre.
fn glow_along(c: f32, glow: f32, per_metre: f32, distance: f32) -> f32 {
    var absorbed = 0.0;
    if (is_finite(c)) {
        absorbed = -log(clamp(c, 1e-6, 1.0)) * per_metre;
    }
    if (absorbed * distance < 1e-5) {
        return glow * distance;
    }
    return glow * (1.0 - exp(-absorbed * distance)) / absorbed;
}

fn emitted_over(m: GpuMaterial, distance: f32) -> vec3<f32> {
    if (!FEATURE_VOLUME_GLOW || all(m.volume_glow.xyz == vec3<f32>(0.0)) || distance <= 0.0 || !is_finite(distance)) {
        return vec3<f32>(0.0);
    }
    let c = m.transmission_color.xyz;
    let g = m.volume_glow;
    return vec3<f32>(glow_along(c.x, g.x, g.w, distance), glow_along(c.y, g.y, g.w, distance), glow_along(c.z, g.z, g.w, distance));
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

/// A shadow ray: where it sets out, its way and how far it goes.
struct ShadowRay {
    origin: vec3<f32>,
    direction: vec3<f32>,
    reach: f32,
};

// intersect.rs's shadow_ray: from `from_point` as it is - every caller has
// stepped it off its surface already, once (Hit::offset, #179) - to the aim
// stepped off its own, stopped just short of it.
fn shadow_ray(from_point: vec3<f32>, from_normal: vec3<f32>, to: vec3<f32>, to_normal: vec3<f32>) -> ShadowRay {
    let start = from_point;
    let aim = offset_at(to, to_normal, 0.0);
    let delta = aim - start;
    let distance = sqrt(dot(delta, delta));
    let direction = select(from_normal, delta / distance, distance > 0.0);
    return ShadowRay(start, direction, distance * (1.0 - 1e-4));
}

// ---- The one walk along a ray (docs/07 D5, D6) -----------------------------
//
// Every ray the transport traces goes through walk_ray(), the one ray
// query, which path() calls from one place and direct_light from a second:
// the driver inlines every function, and a query copied at each place a ray
// was wanted - with what follows it, the opacity a program drives, the
// clear glass, the medium - had made the kernel's cold compile grow with
// every port. A function that needs a walk halfway hands back what to walk
// and where it stands, path() makes the walk at its place in its loop, and
// the function takes up again from there with what the walk found (`Walk`,
// `WalkEnd`). The light sample walks its shadow rays itself, where its stack
// is at hand (walk_ray's mode a constant there, so only the shadow ray's
// handling is copied): carried over a turn of path()'s loop, the stack and a
// source's numbers made the kernel two to three times slower to render. No
// walk draws from the sampler.

// What a walk is for.
// WALK_FIRST: the closest surface of any part within `reach`
// (Intersector::intersect): a path's ray - a walk's flight traced only as
// far as it goes (ic:998) - the floor's mirror ray's first, a bevel's probe.
const WALK_FIRST: u32 = 6u;
// WALK_SHADOW: a shadow ray (shadow_transmittance_through, ic:3308-3447),
// the parts that cast lamp `blockers`' shadow by its shadow links the
// blockers (every part for NO_LIGHT, and for a lamp without shadow links):
// where no glass is passed, nothing is cut out and every part is a blocker,
// one any-hit query among the parts that cast shadows, all or nothing
// (Intersector::occluded, Embree's MASK_SHADOW); else the closest surfaces
// along the shadow ray one after another, over every part, at most 64 -
// clear glass `glass` takes in taking its tint at each face and its volume
// between the way in and the way out of one solid (one at a time, as the
// processor pairs them), a part that casts no shadow, or none of the
// lamp's, passed, anything else letting by what its opacity does not stop
// (a wall nothing, a cutout's hole the rest; crossing_opacity). With no
// glass let by and no lamp, blocked_towards (cutout_transmittance, ic:3333,
// or Intersector::occluded where nothing is cut out). Or, `in_medium`,
// medium_transmittance (ic:4186) through the uniform medium of extinction
// `sigma` of the part `part` the ray sets out inside: its transmittance over
// every stretch inside, its own surface passed (it bends nothing), and a
// part that casts no shadow; anything else in the way of the parts that
// cast lamp `blockers`' shadow stops it (the part is the instance here,
// scene.rs, as the processor keys its media).
const WALK_SHADOW: u32 = 0u;
// WALK_PAST: walk_past (ic:3567), as a shadow ray of lamp `blockers` that
// is never one any-hit query: within `reach` every surface it lets light by
// passed, black where anything else stops it (an opacity of 0.999 or more
// at once); with `glows`, stopping at the first glowing part (an instance
// with leaves among the glowing triangles).
const WALK_PAST: u32 = 1u;
// WALK_COVERAGE: camera_coverage's walk (ic:3467), the light through every
// surface past the first cutout the camera sees, of opacity `first`; a
// shadow catcher ends it, and a part the camera does not see is passed.
const WALK_COVERAGE: u32 = 2u;
// WALK_MEDIA: enclosing_media (ic:248-296), the media a path starts inside,
// by counting along the line back from its first ray's origin, away from
// where it looks (#169: that line usually meets nothing, where the one ahead
// crosses everything in view), through everything - opaque surfaces too -
// up to 32 surfaces, the crossings of each medium's boundary: an odd count
// of one solid's (GpuInstance::solid, a shell split by material one) is
// inside it.
const WALK_MEDIA: u32 = 3u;
// WALK_GLOW: SeenFrom::weight_of_glow's walk (ic:2730), the first surface
// past what a shadow ray passes - a part that casts no shadow, clear glass,
// a cutout - which must face the way.
const WALK_GLOW: u32 = 4u;
// WALK_NEARNESS: nearness_at's walk (ic:3232), within `reach` every part but
// a lamp's stand-in and the floor itself in the way, a cutout by its opacity
// (one a program drives as solid).
const WALK_NEARNESS: u32 = 5u;

struct Walk {
    mode: u32,
    origin: vec3<f32>,
    direction: vec3<f32>,
    reach: f32,
    glass: u32,
    blockers: u32,
    in_medium: bool,
    part: u32,
    sigma: vec3<f32>,
    glows: bool,
    first: f32,
    // A shadow ray that passes the shadow-catching floors the bounces pass:
    // the sky's light sample where it sees past them (sky_past_floors).
    past_floors: bool,
};

// What a walk found: the light it lets through (a shadow ray's, the light
// past a part's; the camera's coverage and the floor's nearness in `x`) and
// whether it passed glass that bends light; whether it stopped where it was
// looking (the light past a part at a glowing part, the camera's coverage
// at a shadow catcher, weight_of_glow at the surface it weighs by), the hit
// it stopped at (the closest, for WALK_FIRST) and that hit's place
// (hit_place); the media a path starts inside.
struct WalkEnd {
    through: vec3<f32>,
    bent: bool,
    stopped: bool,
    hit: RayIntersection,
    place: Surface,
    media: Media,
};

fn walk_new(mode: u32, origin: vec3<f32>, direction: vec3<f32>, reach: f32) -> Walk {
    return Walk(mode, origin, direction, reach, GLASS_SOLID, NO_LIGHT, false, NO_SOLID, vec3<f32>(0.0), false, 0.0, false);
}

// A shadow ray's walk (WALK_SHADOW) along `ray`.
fn shadow_walk_of(glass: u32, ray: ShadowRay, in_medium: bool, part: u32, sigma: vec3<f32>, blockers: u32) -> Walk {
    var w = walk_new(WALK_SHADOW, ray.origin, ray.direction, ray.reach);
    w.glass = glass;
    w.blockers = blockers;
    w.in_medium = in_medium;
    w.part = part;
    w.sigma = sigma;
    return w;
}

// shadow_transmittance_through's walk between two points.
fn shadow_through_walk(glass: u32, from_point: vec3<f32>, from_normal: vec3<f32>, to: vec3<f32>, to_normal: vec3<f32>, blockers: u32) -> Walk {
    return shadow_walk_of(glass, shadow_ray(from_point, from_normal, to, to_normal), false, NO_SOLID, vec3<f32>(0.0), blockers);
}

// How much light gets from one point to another, and whether it came
// through glass that bends light on the way.
struct Visibility {
    through: vec3<f32>,
    bent: bool,
};

// A shadow ray's walk made at once, for the test kernels (tests/media.rs);
// the transport makes its walks at path()'s one place and direct_light's.
fn shadow_walk(glass: u32, ray: ShadowRay, in_medium: bool, part: u32, sigma: vec3<f32>, blockers: u32) -> Visibility {
    let end = walk_ray(shadow_walk_of(glass, ray, in_medium, part, sigma, blockers));
    return Visibility(end.through, end.bent);
}

fn walk_ray(w: Walk) -> WalkEnd {
    var out: WalkEnd;
    out.through = vec3<f32>(0.0);
    out.bent = false;
    out.stopped = false;
    out.media.len = 0u;
    let mode = w.mode;
    let direction = w.direction;
    let past = FEATURE_PASSES_PARTS && mode == WALK_PAST;
    let lets_by = mode == WALK_SHADOW || past;
    var glass = w.glass;
    if (!FEATURE_PANES || (frame.glass_flags & SCENE_PANES) == 0u) {
        glass = GLASS_SOLID;
    }
    // The lamp's shadow links, where it has any.
    var linked = false;
    var lamp: GpuLight;
    if (FEATURE_SHADOW_LINKS && lets_by && w.blockers != NO_LIGHT) {
        lamp = scene_lights[w.blockers];
        linked = (lamp.flags & (LF_SHADOW_LINKS_ONLY | LF_SHADOW_LINKS_ALL_BUT)) != 0u;
    }
    let in_medium = FEATURE_SCATTERING && mode == WALK_SHADOW && w.in_medium;
    let glows = FEATURE_EMITTERS && past && w.glows;
    let solid = mode == WALK_SHADOW && glass == GLASS_SOLID && (!FEATURE_CUTOUTS || (frame.glass_flags & SCENE_CUTOUTS) == 0u) && !linked && !in_medium;
    let flags = select(RAY_FLAG_FORCE_OPAQUE, RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_TERMINATE_ON_FIRST_HIT, solid);
    let mask = select(MASK_SURFACE, select(MASK_SHADOW | MASK_FLOOR, MASK_SHADOW, FEATURE_FLOOR && w.past_floors), solid);
    let sigma = w.sigma;
    let majorant = max(max(sigma.x, sigma.y), sigma.z);
    var origin = w.origin;
    var remaining = w.reach;
    // What gets through: a colour for a shadow ray and the light past a part;
    // the camera's coverage and the floor's nearness in every lane alike.
    var through = vec3<f32>(1.0);
    if (FEATURE_CUTOUTS && mode == WALK_COVERAGE) {
        through = vec3<f32>(1.0 - clamp(w.first, 0.0, 1.0));
    }
    var bent = false;
    var along = 0.0;
    var inside = NO_SOLID;
    var since = 0.0;
    var in_part = true;
    // The media's count: each boundary's solid, its material and how often
    // the ray crossed it.
    var solids: array<u32, 4>;
    var materials: array<u32, 4>;
    var counts: array<u32, 4>;
    var used = 0u;
    let limit = select(MAX_CUTOUT_CROSSINGS, 32u, mode == WALK_MEDIA);
    for (var crossing = 0u; crossing < limit; crossing++) {
        if (FEATURE_CUTOUTS && mode == WALK_COVERAGE && through.x < 1e-4) {
            out.through = vec3<f32>(1.0);
            return out;
        }
        // ---- The transport's one ray query ----
        var rq: ray_query;
        rayQueryInitialize(&rq, scene_tlas, RayDesc(flags, mask, 0.0, remaining, origin, direction));
        rayQueryProceed(&rq);
        let hit = rayQueryGetCommittedIntersection(&rq);
        out.hit = hit;
        if (mode == WALK_FIRST) {
            return out;
        }
        if (in_medium && in_part && majorant > 0.0) {
            through = through * md_survive(sigma, select(remaining, hit.t, hit.kind != RAY_QUERY_INTERSECTION_NONE));
        }
        if (hit.kind == RAY_QUERY_INTERSECTION_NONE) {
            if (lets_by) {
                out.through = through;
                out.bent = bent;
                return out;
            }
            if (FEATURE_CUTOUTS && mode == WALK_COVERAGE) {
                out.through = vec3<f32>(1.0 - through.x);
                return out;
            }
            // The floor's nearness and the media are worked out below; the
            // glowing surface weighed by is not there.
            break;
        }
        if (solid) {
            return out;
        }
        // Where nothing is cut out, no glass is let by, no lamp has shadow
        // links, nothing scatters, no part is walked past and neither the
        // media nor a glow's surface is asked for, every walk is the one
        // query above: what follows is left out of such a variant.
        if (!(FEATURE_PANES || FEATURE_CUTOUTS || FEATURE_SHADOW_LINKS || FEATURE_SCATTERING || FEATURE_PASSES_PARTS || FEATURE_GLASS || FEATURE_EMITTERS)) {
            return out;
        }
        let inst = scene_instances[hit.instance_custom_data];
        let raw = hit_place(hit);
        out.place = raw;
        let backface = dot(direction, raw.geometric) > 0.0;
        let g = select(raw.geometric, -raw.geometric, backface);
        let beyond = offset_at(raw.position, -g, raw.magnitude);
        if (FEATURE_GLASS && mode == WALK_MEDIA) {
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
            origin = beyond;
            continue;
        }
        if (FEATURE_EMITTERS && mode == WALK_GLOW) {
            if ((inst.flags & INSTANCE_CASTS_SHADOWS) == 0u || (scene_materials[inst.material].glass & GLASS_PANE) != 0u || cut_out(inst.material)) {
                origin = beyond;
                continue;
            }
            out.stopped = !backface;
            return out;
        }
        if (FEATURE_PASSES_PARTS && mode == WALK_NEARNESS) {
            if ((inst.flags & INSTANCE_STAND_IN) == 0u && inst.catcher == NO_CATCHER) {
                var opacity = scene_materials[inst.material].geometry_opacity;
                if ((scene_programs[inst.material].flags & PROGRAM_CUTS) != 0u) {
                    opacity = 1.0;
                }
                through *= 1.0 - clamp(opacity, 0.0, 1.0);
                if (through.x < 1e-3) {
                    out.through = vec3<f32>(1.0);
                    return out;
                }
            }
            let left = remaining - hit.t;
            if (left <= 0.0) {
                break;
            }
            origin = beyond;
            remaining = left;
            continue;
        }
        let coverage = FEATURE_CUTOUTS && mode == WALK_COVERAGE;
        if (coverage && inst.catcher != NO_CATCHER) {
            out.through = vec3<f32>(1.0 - through.x);
            out.stopped = true;
            return out;
        }
        // A shadow ray, the light past a part, or the camera's coverage
        // from here on.
        if (glows && inst.emitter != EMITTER_NONE) {
            out.through = through;
            out.stopped = true;
            out.bent = bent;
            return out;
        }
        let open = (inst.flags & INSTANCE_CASTS_SHADOWS) == 0u || (linked && !lamp_casts(lamp, inst.instance)) || (FEATURE_FLOOR && w.past_floors && (inst.flags & INSTANCE_FLOOR_PASSES) != 0u);
        let at = along + hit.t;
        let m = scene_materials[inst.material];
        let bends = (m.glass & GLASS_BENDS) != 0u;
        // Whether the surface lets by what its opacity does not stop: the
        // camera's coverage any it does not pass unseen; a shadow ray any
        // that casts its shadow and is not clear glass it passes.
        var opaque = false;
        if (coverage) {
            opaque = !(FEATURE_UNSEEN && (inst.flags & INSTANCE_NO_CAMERA) != 0u);
        } else if (in_medium) {
            if (inst.solid != w.part && !open) {
                return out;
            }
            if (inst.solid == w.part) {
                in_part = !in_part;
            }
        } else if ((m.glass & GLASS_PANE) != 0u && pane_passes(glass, bends) && !open) {
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
        } else {
            opaque = !open;
        }
        if (opaque) {
            let opacity = crossing_opacity(hit, direction, inst.material);
            if (past && opacity >= 0.999) {
                out.bent = bent;
                return out;
            }
            through *= 1.0 - clamp(opacity, 0.0, 1.0);
        }
        if (coverage) {
            origin = beyond;
            continue;
        }
        if (!in_medium && max(max(through.x, through.y), through.z) < 1e-3) {
            out.bent = past && bent;
            return out;
        }
        remaining -= select(max(hit.t, 1e-6), hit.t, in_medium);
        if (remaining <= 0.0) {
            out.through = through;
            out.bent = bent;
            return out;
        }
        origin = beyond;
        along = at;
    }
    if (FEATURE_CUTOUTS && mode == WALK_COVERAGE) {
        out.through = vec3<f32>(1.0);
    } else if (FEATURE_PASSES_PARTS && mode == WALK_NEARNESS) {
        out.through = vec3<f32>(1.0 - through.x);
    } else if (FEATURE_GLASS && mode == WALK_MEDIA) {
        for (var j = 0u; j < used; j++) {
            if ((counts[j] & 1u) == 1u) {
                let m = scene_materials[materials[j]];
                out.media.media[out.media.len] = Medium(solids[j], m.medium_priority, m.base_ior, materials[j], false, vec3<f32>(0.0));
                out.media.len += 1u;
            }
        }
    }
    return out;
}

// SeenFrom::weight: the weight of light a bounce meets past glass it saw,
// where `straight` is the density the light sample at the surface gave the
// same light along the way the bounce left. Shared with the sample where
// its straight line gets there through clear glass; where something stops
// it, the bounce takes all of it. seen_mis is the shared weight, 1 where no
// shadow ray asks (below 1, or not a number, its shadow ray seen_walk's,
// the weight then seen_weight_past), and `reach` how far along the way the
// light is (PDF_INFINITE: past everything).
fn seen_mis(seen: SeenFrom, straight: f32) -> f32 {
    if (!(straight > 0.0) || seen.density >= PDF_INFINITE) {
        return 1.0;
    }
    let weight = mis_power(seen.density, straight);
    if (weight >= 1.0) {
        return 1.0;
    }
    return weight;
}

fn seen_walk(seen: SeenFrom, reach: f32) -> Walk {
    var far = frame.shadow_far;
    if (reach < PDF_INFINITE) {
        far = reach * (1.0 - 1e-3);
    }
    let target_point = seen.position + seen.direction * far;
    let from_point = offset_at(seen.position, seen.normal, seen.magnitude);
    return shadow_through_walk(GLASS_CLEAR, from_point, seen.normal, target_point, -seen.direction, NO_LIGHT);
}

fn seen_weight_past(weight: f32, open: WalkEnd) -> f32 {
    return select(weight, 1.0, all(open.through == vec3<f32>(0.0)));
}

// mis_power of a bounce's density `pdf` against a light sample's `density`
// per unit area of a point `distance_squared` away, met at `cos` to its
// normal (area_to_solid_angle_pdf, whose infinity weighs the bounce
// nothing).
fn mis_against_area(pdf: f32, density: f32, distance_squared: f32, cos: f32) -> f32 {
    let c = abs(cos);
    if (c <= 1e-9) {
        return 0.0;
    }
    return mis_power(pdf, density * distance_squared / c);
}

// The surface a light sample of the glowing triangles is weighed by: its
// part's first leaf slot (`emitter`), the triangle, the point and its
// geometric normal; not ok where there is none to weigh by.
struct GlowTarget {
    ok: bool,
    first: u32,
    primitive: u32,
    position: vec3<f32>,
    geometric: vec3<f32>,
};

// SeenFrom::weight_of_glow's walk (ic:2730, WALK_GLOW): from the surface a
// seeing bounce left, along the way it left; a glowing part met past glass
// the bounce saw is weighed against the light sample there as the surface
// it finds' (glow_target_of).
fn glow_walk(seen: SeenFrom) -> Walk {
    return walk_new(WALK_GLOW, offset_at(seen.position, seen.normal, seen.magnitude), seen.direction, 3.0e38);
}

fn glow_target_of(found: WalkEnd) -> GlowTarget {
    var none: GlowTarget;
    none.ok = false;
    if (!found.stopped) {
        return none;
    }
    return GlowTarget(true, scene_instances[found.hit.instance_custom_data].emitter, found.hit.primitive_index, found.place.position, found.place.geometric);
}

// Whether a shadow ray passes a surface of `material` as a cutout
// (SceneSnapshot::opacity_of below 0.999, or a program's).
fn cut_out(material: u32) -> bool {
    return scene_materials[material].geometry_opacity < 0.999 || (scene_programs[material].flags & PROGRAM_CUTS) != 0u;
}

// ---- Cutouts a program drives ----------------------------------------------

// What the one place a program runs is asked for (path()): nothing, the
// material at the segment's hit, or a cutout's opacity where a walk
// crossed it.
const WANT_NONE: u32 = 0u;
const WANT_MAIN: u32 = 1u;
const WANT_CROSSING: u32 = 2u;

// Where a segment of a path stands (path()): asking which media the path
// starts inside; setting out, its flight drawn; its ray traced; the lamps
// met and the sky, the walk, the surface; a floor met; the medium crossed,
// stand-ins and boundaries passed; the rounded edges and the program; at
// the hit, its material known - the cutout and the camera's
// coverage, the coverage known, the light past a part, a glow; then the
// vertex and its light sample.
const PH_MEDIA: u32 = 0u;
const PH_START: u32 = 1u;
const PH_RAY: u32 = 2u;
const PH_TRACED: u32 = 3u;
const PH_FLOOR: u32 = 4u;
const PH_BODY: u32 = 5u;
const PH_BEVEL: u32 = 6u;
const PH_HIT: u32 = 7u;
const PH_COVERED: u32 = 8u;
const PH_PAST: u32 = 9u;
const PH_GLOW: u32 = 10u;

// settings.rs's DEFAULT_PIXEL_SPREAD: the footprint a shadow ray's program
// is read with.
const DEFAULT_PIXEL_SPREAD: f32 = 0.001;

// How many crossings of cutouts a program drives one segment remembers:
// past them a crossing is solid.
const MAX_CUT_PROGRAMS: u32 = 32u;

// The opacities of the cutouts a program drives that this segment's walks
// have crossed, in the order crossed; how many are known; the place in that
// order of the next crossing this pass meets; and the crossing still to be
// worked out, if one is.
var<private> cut_known_opacity: array<f32, 32>;
var<private> cut_known: u32;
var<private> cut_next: u32;
var<private> cut_wanted: bool;
var<private> cut_hit: RayIntersection;
var<private> cut_direction: vec3<f32>;

// SceneSnapshot::opacity_of at a crossing a walk makes along `direction`:
// the material's own number, or for a cutout a program drives, the
// program's at the point - known from an earlier pass of this segment, or
// asked for, when this pass is thrown away and run again once it is known
// (path()); solid for now.
fn crossing_opacity(hit: RayIntersection, direction: vec3<f32>, material: u32) -> f32 {
    if (!FEATURE_CUT_PROGRAMS || (scene_programs[material].flags & PROGRAM_CUTS) == 0u) {
        return scene_materials[material].geometry_opacity;
    }
    let k = cut_next;
    cut_next += 1u;
    if (k < cut_known) {
        return cut_known_opacity[k];
    }
    if (!cut_wanted && cut_known < MAX_CUT_PROGRAMS) {
        cut_wanted = true;
        cut_hit = hit;
        cut_direction = direction;
    }
    return 1.0;
}

// The surface a walk crossed, as a shadow ray's program reads it
// (Hit::facing_forward): both normals turned to the side the ray came from,
// and nothing more.
fn crossing_surface(hit: RayIntersection, direction: vec3<f32>) -> Surface {
    var surf = surface_of(hit);
    surf.backfacing = false;
    if (dot(direction, surf.geometric) > 0.0) {
        surf.geometric = -surf.geometric;
        surf.shading = -surf.shading;
        surf.backfacing = true;
    }
    return surf;
}

// camera_coverage (ic:3467): how much of a camera ray's pixel sample is
// covered, where the first surface it meets is a cutout of `first` opacity
// and the ray carries on from `start` - one less the light that gets
// through every surface along it (WALK_COVERAGE, its `through.x`); and
// whether a shadow catcher ends the walk (`stopped`), whose own coverage is
// its shadow's, which only the path finds.
fn coverage_walk(start: vec3<f32>, direction: vec3<f32>, first: f32) -> Walk {
    var w = walk_new(WALK_COVERAGE, start, direction, 3.0e38);
    w.first = first;
    return w;
}

// Whether a part of instance flags `flags` is unseen by the ray a path is
// on (ic:1352's unseen, by its Seen): the camera's at its first segment;
// the floor's mirror ray at its first, which is the camera's view and a
// reflection both; past those each bounce by its kind (`ray`), the split
// reflection's a glossy one.
fn unseen_by(ray: u32, bounce: u32, in_floor: bool, flags: u32) -> bool {
    if (in_floor && bounce == 0u) {
        return (flags & (INSTANCE_NO_CAMERA | INSTANCE_NO_GLOSSY)) != 0u;
    }
    switch ray {
        case RAY_CAMERA: {
            return bounce == 0u && (flags & INSTANCE_NO_CAMERA) != 0u;
        }
        case RAY_DIFFUSE: {
            return (flags & INSTANCE_NO_DIFFUSE) != 0u;
        }
        case RAY_TRANSMISSION: {
            return (flags & INSTANCE_NO_TRANSMISSION) != 0u;
        }
        default: {
            return (flags & INSTANCE_NO_GLOSSY) != 0u;
        }
    }
}

// ---- The light past a part its shadow rays pass ----------------------------

/// walk_past (ic:3567, WALK_PAST): from just past the surface at `place`
// along `direction`, within `reach` (PDF_INFINITE: past everything), every
// surface a shadow ray of lamp `blockers` lets light by passed - a part that
// casts none of its shadow (every part a blocker for NO_LIGHT), clear glass
// `glass` takes in, a cutout's holes (crossing_opacity) - and black where
// anything else stops it; with `glows`, stopping at the first glowing part.
fn walk_past_of(glass: u32, place: Surface, direction: vec3<f32>, reach: f32, blockers: u32, glows: bool) -> Walk {
    let g0 = select(place.geometric, -place.geometric, dot(direction, place.geometric) > 0.0);
    var w = walk_new(WALK_PAST, offset_at(place.position, -g0, place.magnitude), direction, select(reach, 3.0e38, reach >= PDF_INFINITE));
    w.glass = glass;
    w.blockers = blockers;
    w.glows = glows;
    return w;
}

// What a function that walks hands back to path(): whether it waits on a
// walk, and which; the turn of its loop it waits at and the step within
// that turn; and a colour it carries over the walk (or, done, hands back).
struct Paused {
    wants: bool,
    walk: Walk,
    i: u32,
    step: u32,
    carried: vec3<f32>,
};

// The light past a part its shadow rays pass, where the path stops at it
// (ic:1753-1801): the part at `place` (`hit`), met along the segment, casts
// no shadow, or none of some lamp's, and the light samples at the surface
// the path left counted the light past it. Unless the path goes on past it
// itself (clear glass it passes): the lamps the surface a seeing bounce
// left, or a mirror's view, meets past it (lamps_past, ic:3659) - each
// whose shadow rays pass the part, through what they let by, weighed as
// meet_and_escape weighs a lamp; and past a part that casts no shadow, what
// a bounce weighed against the light samples would have met there
// (light_past, ic:3713): the sky, or the first glowing part past what a
// shadow ray passes, weighed by the density the bounce left by (that of the
// lobes that pass glass where it passed glass that bends light). One walk
// a light (walk_past), in one loop, the sky's and the glowing parts' last;
// each clamped as light found past the bounce before. Carrying wavelengths,
// a light's colour and its lobes' are read at them from one place at the
// end of its turn (ic:1775, 1793-1794).
//
// Its walks are path()'s: it hands one back where it wants it, and is
// called again from turn `start` of its loop at `step` - 1 with walk_past's
// `found`, 2 with the shadow ray of SeenFrom::weight's `found` and
// walk_past's light `carried` - until it wants none.
fn light_past_part(st: ptr<function, PathState>, hit: RayIntersection, place: Surface, material: GpuMaterial, start: u32, step: u32, carried: vec3<f32>, found: WalkEnd) -> Paused {
    var out: Paused;
    out.wants = false;
    let inst = scene_instances[hit.instance_custom_data];
    let glass = (*st).through_glass;
    let pane = FEATURE_PANES && (material.glass & GLASS_PANE) != 0u;
    let casts = (inst.flags & INSTANCE_CASTS_SHADOWS) != 0u;
    if (pane && (pane_passes(glass, (material.glass & GLASS_BENDS) != 0u) || (glass == GLASS_UNBENT && !casts))) {
        return out;
    }
    let seen = (*st).seen;
    let bounce = (*st).bounce;
    let ceiling = light_ceiling((*st).specular_chain, select(bounce - 1u, 0u, bounce == 0u), (*st).roulette);
    let lamps = select(0u, frame.light_count, FEATURE_LAMPS && seen.valid);
    // The sky and the glowing parts: past a part that casts no shadow, off
    // a bounce a light sample stands in for, but not past glass a glossy
    // bounce saw, and only where the lights are sampled.
    let past = !casts && !(*st).specular_chain && (*st).prev_pdf < PDF_INFINITE && !(seen.valid && seen.met)
        && (frame.lamp_flags & LAMPS_UNSAMPLED) == 0u;
    let origin = (*st).origin;
    let direction = (*st).direction;
    for (var i = start; i <= lamps; i++) {
        let at = select(0u, step, i == start);
        let is_sky = i == lamps;
        var walk_glass = glass;
        var reach = PDF_INFINITE;
        var blockers = NO_LIGHT;
        var emitted = vec3<f32>(0.0);
        var lobes = vec3<f32>(1.0);
        var light: GpuLight;
        var shadowed = true;
        if (is_sky) {
            if (!past) {
                break;
            }
        } else {
            light = scene_lights[i];
            if (!seen_meets(seen, light)) {
                continue;
            }
            shadowed = light_has(light, LF_SHADOW);
            if (casts && shadowed && lamp_casts(light, inst.instance)) {
                continue;
            }
            let met = light_met_along(light, origin, direction);
            if (!met.ok || (!met.infinite && met.t < hit.t)) {
                continue;
            }
            if (!met.infinite) {
                reach = (met.t - hit.t) * (1.0 - 1e-3);
            }
            emitted = met.radiance;
            lobes = select(seen.glossy, vec3<f32>(1.0), light_has(light, LF_DIFFUSE));
            walk_glass = select(frame.shadow_glass, glass, seen.sees_glass);
            blockers = i;
        }
        // What walk_past found: the light past the surfaces it passed,
        // whether it passed glass that bends light, and whether it stopped
        // at a glowing part it was looking for (`found.hit`).
        var through = vec3<f32>(1.0);
        var bent = false;
        var stopped = false;
        if (shadowed) {
            if (at == 0u) {
                out.wants = true;
                out.walk = walk_past_of(walk_glass, place, direction, reach, blockers, is_sky && (*st).receiver);
                out.i = i;
                out.step = 1u;
                return out;
            }
            if (at == 1u) {
                through = found.through;
                bent = found.bent;
                stopped = found.stopped;
            } else {
                through = carried;
            }
        }
        if (all(through == vec3<f32>(0.0))) {
            continue;
        }
        var weight = 1.0;
        // Carrying wavelengths: the lobes' colour and the light's, before
        // the throughput, for the end of this turn.
        let spectral = FEATURE_SPECTRAL && spectral_on;
        var lobes4 = vec3<f32>(1.0);
        var light4 = vec3<f32>(0.0);
        if (!is_sky) {
            // SeenFrom::weight_of_lamp.
            if (seen.density < PDF_INFINITE) {
                let chosen = lamps_selection_pdf_at(frame.light_hierarchy != 0u, seen.position, seen.normal, seen.two_sided, i);
                let straight = light_density_along(light, seen.position, seen.direction) * chosen;
                if (seen.met) {
                    let from_seen = light_met_along(light, seen.position, seen.direction);
                    weight = seen_mis(seen, straight);
                    if (!(weight >= 1.0)) {
                        if (at < 2u) {
                            out.wants = true;
                            out.walk = seen_walk(seen, select(PDF_INFINITE, from_seen.t, from_seen.ok && !from_seen.infinite));
                            out.i = i;
                            out.step = 2u;
                            out.carried = through;
                            return out;
                        }
                        weight = seen_weight_past(weight, found);
                    }
                } else {
                    weight = mis_power(seen.density, straight);
                }
            }
            if (!spectral) {
                (*st).radiance += clamp_light((*st).throughput * lobes * (emitted * through * weight), ceiling);
                continue;
            }
            lobes4 = lobes;
            light4 = emitted * through * weight;
        } else {
            let pdf = select((*st).prev_pdf, min((*st).prev_pdf, (*st).pass_pdf), bent);
            if (!stopped) {
                // Each through cy_exact: the driver otherwise rounds what
                // follows otherwise where a dome of a size is a part of the
                // kernel than where it is not (tests/variants.rs's
                // which_part_left_out_...).
                let sky = cy_exact3(past_the_scene(false, false, false, origin, direction));
                let pdf_light = cy_exact(env_pdf_from(origin, direction, (*st).sky_facing));
                if (!spectral) {
                    (*st).radiance += clamp_light((*st).throughput * (sky * through * mis_power(pdf, pdf_light)), ceiling);
                }
                light4 = sky * through * mis_power(pdf, pdf_light);
            } else if (FEATURE_EMITTERS) {
                let next = found.hit;
                let stop = found.place;
                if (dot(direction, stop.geometric) <= 0.0) {
                    let first = scene_instances[next.instance_custom_data].emitter;
                    let density = emitters_density((*st).receiver_position, (*st).receiver_normal, (*st).receiver_two_sided, first, next.primitive_index);
                    if (density > 0.0) {
                        let to = stop.position - (*st).receiver_position;
                        let glow = scene_emitter_nodes[scene_emitter_leaves[first + next.primitive_index]].radiance.xyz;
                        if (!spectral) {
                            (*st).radiance += clamp_light((*st).throughput * (glow * through * mis_against_area(pdf, density, dot(to, to), dot(direction, stop.geometric))), ceiling);
                        }
                        light4 = glow * through * mis_against_area(pdf, density, dot(to, to), dot(direction, stop.geometric));
                    }
                }
            }
        }
        if (spectral) {
            // The lobes' colour, then the light's (a lamp's; the sky's or a
            // glowing part's under no lobes), read at the wavelengths from
            // one place: throughput * reflectance(lobes) * unbounded(light).
            var lit = (*st).tp4;
            for (var k = 0u; k < (2u | cy_zero); k++) {
                lit = lit * spectral_of(select(lobes4, light4, k == 1u), k == 1u);
            }
            (*st).rad4 += cy_exact4(clamp_light4(lit, ceiling));
        }
    }
    return out;
}

// A direction drawn as ambient occlusion draws it (occlusion_at,
// nearness_at): cosine-weighted about `normal`, from the light-sample
// dimensions of whatever bounce the sampler is on.
fn occlusion_direction(normal: vec3<f32>, s: ptr<function, Sampler>) -> vec3<f32> {
    let f = frame_of(normal);
    return to_world(f, sample_cosine_hemisphere(sampler_light_sample(s)));
}

// nearness_at (ic:3232), for a floor whose reflection fades by distance in
// a scene with parts that cast no shadow: how much the parts standing on the
// floor close in within `radius`, one cosine-weighted direction drawn as
// occlusion_at draws it (WALK_NEARNESS; the nearness its `through.x`).
fn nearness_walk(position: vec3<f32>, magnitude: f32, normal: vec3<f32>, radius: f32, s: ptr<function, Sampler>) -> Walk {
    let direction = occlusion_direction(normal, s);
    return walk_new(WALK_NEARNESS, offset_at(position, normal, magnitude), direction, max(radius, 1e-3));
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
    // What Russian roulette has scaled the throughput up by (ic's roulette),
    // which the clamps leave out.
    roulette: f32,
    transmission_bounces: u32,
    // Passes through glass (ic:847-861's glass_crossings): counted against
    // the transmission limit and not against the bounces (#183), so the
    // bounce limit is `bounce - glass_crossings`.
    glass_crossings: u32,
    through_glass: u32,
    pass_pdf: f32,
    media: Media,
    reflected: bool,
    in_floor: bool,
    fresh: bool,
    // Where the path last sampled the glowing parts from, if it did where
    // it last scattered (ic:850's emitters_from): a glow its bounce then
    // hits was one of two ways of finding that light, weighed against the
    // other as seen from there. Its point, its geometric normal on the side
    // the path left by, and whether it transmits (chose both ways).
    receiver: bool,
    receiver_position: vec3<f32>,
    receiver_normal: vec3<f32>,
    receiver_two_sided: bool,
    // The one wavelength the path carries from the first dispersive glass
    // it met on (ic:833-836), nought before.
    wavelength: f32,
    // Carrying wavelengths (spectral_on), the throughput and the radiance
    // at each of the four in place of the three channels, and whether the
    // hero carries on alone (ic:786-793).
    tp4: vec4<f32>,
    rad4: vec4<f32>,
    alone: bool,
    // The random walk (ic:815-832): the steps it has taken through media
    // that scatter, and the medium whose last collision sampled the lights
    // directly (its solid, NO_SOLID for none) with the density its phase
    // function chose the way on by - until the path meets anything but
    // that medium's own surface, the sky it escapes to is weighed by it.
    walk_steps: u32,
    lit_from: u32,
    lit_pdf: f32,
    // The walk under a surface the path came in on, guided back towards
    // where it came in (ic:822's WalkGuide; NO_WALK for none).
    guide: WalkGuide,
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
    st.roulette = 1.0;
    st.transmission_bounces = 0u;
    st.glass_crossings = 0u;
    st.through_glass = GLASS_SOLID;
    st.pass_pdf = PDF_INFINITE;
    st.media.len = 0u;
    st.reflected = reflected;
    st.in_floor = in_floor;
    st.fresh = true;
    st.receiver = false;
    st.wavelength = 0.0;
    st.tp4 = vec4<f32>(1.0);
    st.rad4 = vec4<f32>(0.0);
    st.alone = false;
    // Measuring what the companions buy, the hero alone from the start.
    if (FEATURE_SPECTRAL && spectral_on && (frame.mode & MODE_ALONE) != 0u) {
        st.tp4 = spectral_hero_alone(st.tp4);
        st.alone = true;
    }
    st.walk_steps = 0u;
    st.lit_from = NO_SOLID;
    st.lit_pdf = 0.0;
    st.guide = md_no_guide();
    return st;
}

// ---- A dome of a size (environment.rs's Dome) ---------------------------
// Under FEATURE_DOME, where the frame says the environment is a dome of a
// size (SCENE_FINITE_DOME): a sphere of dome_radius about (0, dome_height, 0), its
// lower cap the ground plane where dome_flatten. Every lookup of the sky
// from a point goes through dome_surface_point, as the processor's does.

fn dome_finite() -> bool {
    return FEATURE_DOME && (frame.glass_flags & SCENE_FINITE_DOME) != 0u;
}

fn dome_centre() -> vec3<f32> {
    return vec3<f32>(0.0, frame.dome_height, 0.0);
}

// Vec3::dot, summed in order: the card would fuse it otherwise.
fn dome_dot(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return cy_exact(cy_exact(a.x * b.x) + cy_exact(a.y * b.y)) + cy_exact(a.z * b.z);
}

struct DomeHit {
    ok: bool,
    point: vec3<f32>,
    normal: vec3<f32>,
    distance: f32,
};

// Dome::surface_point: where the ray (origin, direction) meets the dome -
// the nearest root past 1e-4 of the sphere, or the ground plane in front
// of it within the dome's footprint where the dome is flattened.
fn dome_surface_point(origin: vec3<f32>, direction: vec3<f32>) -> DomeHit {
    var none: DomeHit;
    none.ok = false;
    let radius = frame.dome_radius;
    let centre = dome_centre();
    let oc = origin - centre;
    let b = dome_dot(oc, direction);
    let c = dome_dot(oc, oc) - cy_exact(radius * radius);
    let discriminant = cy_exact(b * b) - c;
    if (discriminant < 0.0) {
        return none;
    }
    let root = cy_sqrt(discriminant);
    let far = -b + root;
    let near = -b - root;
    var t: f32;
    if (near > 1e-4) {
        t = near;
    } else if (far > 1e-4) {
        t = far;
    } else {
        return none;
    }
    if (frame.dome_flatten != 0u && direction.y < -1e-6) {
        let t_plane = cy_div(-origin.y, direction.y);
        if (t_plane > 1e-4 && t_plane < t) {
            let point = origin + cy_exact3(direction * t_plane);
            let offset = point - centre;
            if (cy_exact(cy_exact(offset.x * offset.x) + cy_exact(offset.z * offset.z)) <= cy_exact(radius * radius)) {
                return DomeHit(true, point, vec3<f32>(0.0, -1.0, 0.0), t_plane);
            }
        }
    }
    let point = origin + cy_exact3(direction * t);
    return DomeHit(true, point, (centre - point) * cy_div(1.0, radius), t);
}

// try_normalize: the vector over its length, where that is a number over
// nought; else none (a zero vector).
fn dome_normalized(v: vec3<f32>) -> vec3<f32> {
    let length_squared = dome_dot(v, v);
    if (!(length_squared > 0.0 && length_squared < 3.4e38)) {
        return vec3<f32>(0.0);
    }
    return v * cy_div(1.0, cy_sqrt(length_squared));
}

// Dome::lookup's direction: the image read towards where the ray meets the
// dome, from its centre; the ray's own where it meets none.
fn dome_lookup(origin: vec3<f32>, direction: vec3<f32>) -> vec3<f32> {
    let hit = dome_surface_point(origin, direction);
    if (!hit.ok) {
        return direction;
    }
    let from_centre = dome_normalized(hit.point - dome_centre());
    return select(from_centre, direction, all(from_centre == vec3<f32>(0.0)));
}

struct FromCentre {
    ok: bool,
    direction: vec3<f32>,
    jacobian: f32,
    distance: f32,
};

// Dome::from_centre: the point of the dome seen from its centre along
// `local`, as seen from `origin` - the direction there, the solid angle a
// patch spans from the centre over what it spans from `origin`, and how far.
fn dome_from_centre(origin: vec3<f32>, local: vec3<f32>) -> FromCentre {
    var none: FromCentre;
    none.ok = false;
    let hit = dome_surface_point(dome_centre(), local);
    if (!hit.ok) {
        return none;
    }
    let offset = hit.point - origin;
    let distance = cy_sqrt(dome_dot(offset, offset));
    if (distance < 1e-4) {
        return none;
    }
    let direction = offset * cy_div(1.0, distance);
    let from_centre_distance = max(hit.distance, 1e-6);
    let cos_centre = max(abs(dome_dot(local, hit.normal)), 1e-6);
    let cos_surface = max(abs(dome_dot(direction, hit.normal)), 1e-6);
    let jacobian = cy_exact(cy_div(cos_centre, cy_exact(from_centre_distance * from_centre_distance)) * cy_div(cy_exact(distance * distance), cos_surface));
    return FromCentre(true, direction, jacobian, distance);
}

// Dome::spread_bound: the most a patch of sky grows by seen from `origin`
// rather than the centre, 1 / (1 - off)^2 for `origin` off the centre by
// `off` of the radius (at most 0.999).
fn dome_spread_bound(origin: vec3<f32>) -> f32 {
    let off = min(cy_div(cy_sqrt(dome_dot(origin - dome_centre(), origin - dome_centre())), frame.dome_radius), 0.999);
    let gap = 1.0 - off;
    return cy_div(1.0, cy_exact(gap * gap));
}

// Where on the film the camera's sample is, which a backplate is mapped to
// (Background::Backplate): set where the camera's ray is made.
var<private> camera_film: vec2<f32>;

// past_the_scene (ic:2831): what a path sees past the scene along
// `direction` - where it escapes, and behind a shadow catcher it meets as
// the camera would see it. Along a specular chain a background colour or a
// backplate replaces the view (SCENE_BACKGROUND_COLOUR, SCENE_BACKPLATE),
// the plate at the film point the path set out from: the camera's sample's,
// and its first glass's reflection's; a floor's reflection traces its path
// from the film's corner (floor_reflection's Vec2::ZERO). Else what the
// camera sees of the environment on its own first segment, else the
// environment.
fn past_the_scene(specular_chain: bool, camera_view: bool, in_floor: bool, origin: vec3<f32>, direction: vec3<f32>) -> vec3<f32> {
    let background = frame.glass_flags & (SCENE_BACKGROUND_COLOUR | SCENE_BACKPLATE);
    if (FEATURE_BACKGROUND && specular_chain && background != 0u) {
        if (FEATURE_BACKPLATE && (background & SCENE_BACKPLATE) != 0u) {
            return plate_bilinear(select(camera_film, vec2<f32>(0.0), in_floor));
        }
        return frame.background;
    }
    // From the path's origin with a dome of a size (radiance_from,
    // background_radiance_from): the image read towards where the ray meets
    // the dome, from its centre.
    var towards = direction;
    if (dome_finite()) {
        towards = dome_lookup(origin, direction);
    }
    return env_radiance(camera_view, towards);
}

// Environment::pdf_facing at `origin`, or with a dome of a size pdf_from:
// the whole sky's density towards where the ray meets the dome, from its
// centre, times the patch's jacobian there; nought where it meets none.
fn env_pdf_from(origin: vec3<f32>, direction: vec3<f32>, facing: vec3<f32>) -> f32 {
    var towards = direction;
    var normal = facing;
    var scale = 1.0;
    if (dome_finite()) {
        let hit = dome_surface_point(origin, direction);
        if (!hit.ok) {
            return 0.0;
        }
        towards = dome_normalized(hit.point - dome_centre());
        if (all(towards == vec3<f32>(0.0))) {
            return 0.0;
        }
        let seen = dome_from_centre(origin, towards);
        if (!seen.ok) {
            return 0.0;
        }
        normal = vec3<f32>(0.0);
        scale = seen.jacobian;
    }
    return env_pdf_sky(towards, normal) * scale;
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
// handed back (`carried`): the background where the camera ray escapes.
// The shadow ray of SeenFrom::weight is path()'s walk: it hands it back, and
// is called again from turn `start` (`resuming`) with what it `found`.
fn meet_and_escape(st: ptr<function, PathState>, hit: RayIntersection, start: u32, resuming: bool, found: WalkEnd) -> Paused {
    var out: Paused;
    out.wants = false;
    let seen = (*st).seen;
    let open = hit.kind == RAY_QUERY_INTERSECTION_NONE;
    var reach = 0.0;
    if (!open) {
        let stand_in = (scene_instances[hit.instance_custom_data].flags & INSTANCE_STAND_IN) != 0u;
        reach = select(hit.t, hit.t * (1.0 + 1e-3) + 1e-5, stand_in);
    }
    let bounce = (*st).bounce;
    let found_at = select(bounce - 1u, 0u, bounce == 0u);
    let ceiling = light_ceiling((*st).specular_chain, found_at, (*st).roulette);
    let lamps = select(0u, frame.light_count, seen.valid);
    var sky = vec3<f32>(0.0);
    for (var i = start; i <= lamps; i++) {
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
            emitted = past_the_scene((*st).specular_chain, bounce == 0u && !(*st).reflected, (*st).in_floor, (*st).origin, (*st).direction);
            if (all(emitted == vec3<f32>(0.0))) {
                break;
            }
            // Straight out of a medium whose last collision sampled the
            // sky too, against the whole sky's density from here, by the
            // phase function's (ic:1281); past glass a glossy bounce saw,
            // against the sky's sample there, straight on the way it left.
            // Whole where the lights are not sampled directly.
            let lit = FEATURE_SCATTERING && (*st).lit_from != NO_SOLID;
            if (lit || (!(*st).specular_chain && (frame.lamp_flags & LAMPS_UNSAMPLED) == 0u)) {
                let met_glass = !lit && seen.valid && seen.met;
                let facing = select(select((*st).sky_facing, seen.sky_facing, met_glass), vec3<f32>(0.0), lit);
                let pdf = env_pdf_from(select((*st).origin, seen.position, met_glass), select((*st).direction, seen.direction, met_glass), facing);
                if (met_glass) {
                    weigh = true;
                    straight = pdf;
                } else {
                    weight = mis_power(select((*st).prev_pdf, (*st).lit_pdf, lit), pdf);
                }
            }
        } else {
            let light = scene_lights[i];
            if (!FEATURE_LAMPS || !seen_meets(seen, light)) {
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
            weight = seen_mis(seen, straight);
            if (!(weight >= 1.0)) {
                if (!(resuming && i == start)) {
                    out.wants = true;
                    out.walk = seen_walk(seen, along);
                    out.i = i;
                    return out;
                }
                weight = seen_weight_past(weight, found);
            }
        }
        if (FEATURE_SPECTRAL && spectral_on) {
            // Every product an add reads through cy_exact4, so a variant
            // rounds as the whole kernel does.
            // The lobes' colour, then the light's, read at the wavelengths
            // from one place.
            var lit = (*st).tp4;
            for (var k = 0u; k < (2u | cy_zero); k++) {
                lit = lit * spectral_of(select(lobes, emitted, k == 1u), k == 1u);
            }
            let contribution = cy_exact4(clamp_light4(lit * weight, ceiling));
            (*st).rad4 += contribution;
            if (is_sky) {
                sky = spectral_rgb(contribution);
            }
        } else {
            let contribution = clamp_light((*st).throughput * lobes * emitted * weight, ceiling);
            (*st).radiance += contribution;
            if (is_sky) {
                sky = contribution;
            }
        }
    }
    out.carried = sky;
    return out;
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
// (sample_environment_light, ic:4279), one of the lamps
// (sample_analytic_lights, ic:4550) and one of the glowing triangles
// (sample_emissive_surfaces, ic:4447), each with its own shadow ray, summed
// before the throughput, as the processor sums them. One pass of the loop
// is one light source, so the three share their evaluation of the stack,
// their shadow ray and their arithmetic. An environment turned down to
// nothing, a scene with no lamp, and one with no glowing triangle a light
// sample takes draw nothing (ic:4299, 4565, 2117). A surface that
// transmits is lit from both sides: it draws the whole sky, chooses lamps
// and glowing triangles on both sides, and its shadow rays leave from the
// side the light is on; the light through clear glass is tinted, and
// through glass that bends light the sky's and the glowing triangles' is
// shared with the bounces that see it.
//
// Or a collision's (medium_direct_light, ic:4123), in a medium whose
// surface bends nothing: one draw of the whole sky seen from the point
// (sample_from), and one lamp chosen by power - not down the hierarchy, a
// point in a medium having no normal to cull by - each weighed by the phase
// function towards it and by what of the medium lies between
// (WALK_SHADOW's medium_transmittance); the sky's share against the phase
// function's density, the lamp's whole; each clamped on its own, after the
// path's throughput. One call of this for both (path's, after the vertex
// is shaded), for the kernel's size: the second call a collision had made
// took the whole kernel's cold compile from 40 s to over 300.
//
// What a vertex leaves for it: at a surface, its stack, frame, the way out,
// the surface, its instance, whether it transmits and the sky it faces; at
// a collision (`medium`), the point (`surf.position`), the way the path was
// going (`wo`, in the world), the medium's part (`instance`), its
// extinction and anisotropy. The path's throughput and the clamp's ceiling
// where it was taken.
struct LightSampleAt {
    wanted: bool,
    medium: bool,
    bsdf: OpenPbr,
    f: Frame,
    wo: vec3<f32>,
    surf: Surface,
    instance: u32,
    transmissive: bool,
    facing: vec3<f32>,
    throughput: vec3<f32>,
    ceiling: f32,
    sigma: vec3<f32>,
    g: f32,
    // Carrying wavelengths: the path's four at the vertex, whether its
    // hero carries on alone, and whether the surface disperses light
    // (ic:2172-2193).
    tp4: vec4<f32>,
    alone: bool,
    disperses: bool,
    // Its draws, taken where it is set up, in the processor's order - the
    // sky's, the lamp's choice and the lamp's, the glowing part's choice and
    // its point (ic:2140-2240, 4290-4350): past the Sobol' dimensions (from
    // bounce 7) each is the next of the open stream, which the processor
    // draws from for the light sample before the bounce, and for a
    // collision's before its turn. The lamp chosen with them.
    u_sky: vec2<f32>,
    choice: LampChoice,
    u_lamp: vec2<f32>,
    u_glow_pick: f32,
    u_glow: vec2<f32>,
};

// The light samples' sum, and each source's apart - the sky's, the lamp's,
// the glowing triangle's, as the stack's value and what scales it, whose
// product is the source's light - with whether it came through the surface
// (bit `source`), which a spectral path through dispersive glass reads
// (ic:2172-2193). The factors rather than their product: an extra reader
// of the product changed how the driver rounded the sum in the kernels
// that have one.
struct Direct {
    total: vec3<f32>,
    values: array<vec3<f32>, 3>,
    scales: array<vec3<f32>, 3>,
    through: u32,
};

fn direct_light(s: ptr<function, Sampler>, here: LightSampleAt) -> Direct {
    let bsdf = here.bsdf;
    let f = here.f;
    let wo = here.wo;
    let surf = here.surf;
    let instance = here.instance;
    let transmissive = here.transmissive;
    let facing = here.facing;
    let medium = FEATURE_SCATTERING && here.medium;
    var direct = vec3<f32>(0.0);
    var parts: Direct;
    for (var source = 0u; source < 3u; source++) {
        let sky = source == 0u;
        let glow = source == 2u;
        var draw: LightDraw;
        var diffuse = true;
        var specular = true;
        var shadow = true;
        var p_select = 1.0;
        var is_shared = false;
        // The lamp whose shadow links say which parts cast its shadow; every
        // part for the sky and the glowing triangles.
        var blockers = NO_LIGHT;
        // A glowing triangle's shadow ray ends at the point chosen on it,
        // moved off it along its normal.
        var glow_point = vec3<f32>(0.0);
        var glow_normal = vec3<f32>(0.0);
        if (glow) {
            if (medium || (frame.glass_flags & SCENE_EMITTERS) == 0u) {
                continue;
            }
            // Chosen from here down the tree, on the bounce's light choice
            // and light sample, as the lamp was.
            let e = emitters_sample(surf.position, surf.geometric, transmissive, here.u_glow_pick, here.u_glow);
            if (!e.ok) {
                continue;
            }
            let to_light = e.position - surf.position;
            let distance_squared = dot(to_light, to_light);
            if (!(distance_squared > 0.0 && is_finite(distance_squared))) {
                continue;
            }
            let direction = to_light * (1.0 / sqrt(distance_squared));
            // It glows from the side its winding faces, to a light sample as
            // to a bounce; seen edge on its density is no number.
            let cos_light = -dot(direction, e.normal);
            if (cos_light <= 1e-9) {
                continue;
            }
            let pdf = e.density * distance_squared / cos_light;
            if (!(pdf > 0.0 && is_finite(pdf))) {
                continue;
            }
            draw = LightDraw(true, direction, e.radiance, 0.0, false, pdf, false);
            glow_point = e.position;
            glow_normal = e.normal;
        } else if (sky) {
            if (!medium && frame.env_intensity <= 0.0) {
                continue;
            }
            // With a dome of a size, the whole sky drawn from its centre and
            // seen from here (sample_from): the direction there, the density
            // times the patch's jacobian, and how far the dome is.
            let finite = dome_finite();
            let e = env_sample_sky(select(facing, vec3<f32>(0.0), finite), here.u_sky);
            if (!e.ok) {
                continue;
            }
            var towards = e.direction;
            var pdf = e.pdf;
            var distance = 0.0;
            if (finite) {
                let seen = dome_from_centre(surf.position, e.direction);
                if (!seen.ok) {
                    continue;
                }
                towards = seen.direction;
                pdf = e.pdf * seen.jacobian;
                distance = seen.distance;
                if (!(pdf > 0.0)) {
                    continue;
                }
            }
            if (pdf <= 0.0 || all(e.radiance == vec3<f32>(0.0))) {
                continue;
            }
            draw = LightDraw(true, towards, e.radiance, distance, !finite, pdf, false);
        } else {
            if (!FEATURE_LAMPS || frame.light_count == 0u) {
                continue;
            }
            // The lamp chosen from this point, facing the side the path is on
            // (both sides for a surface that transmits), and its own variates:
            // both where the sample was set up.
            let choice = here.choice;
            if (!choice.ok) {
                continue;
            }
            let light = scene_lights[choice.index];
            // Light linking: a lamp kept to other parts lights nothing here.
            if (!lamp_lights(light, instance)) {
                continue;
            }
            draw = lamp_sample(light, surf.position, here.u_lamp);
            if (!draw.ok || all(draw.radiance == vec3<f32>(0.0)) || (!draw.is_delta && draw.pdf <= 0.0)) {
                continue;
            }
            diffuse = light_has(light, LF_DIFFUSE);
            specular = light_has(light, LF_SPECULAR);
            shadow = light_has(light, LF_SHADOW);
            p_select = choice.probability;
            blockers = choice.index;
            // A collision's lamp lights by its diffuse visibility, with a
            // density.
            if (medium && !(diffuse && (draw.is_delta || draw.pdf > 0.0))) {
                continue;
            }
            // Shared with the bounces that meet it (ic:4686).
            is_shared = !medium && (frame.lamp_flags & LAMPS_SEEN) != 0u && frame.shadows != 0u && !draw.is_delta && lamp_met_by_bounces(light, instance);
        }
        // Lit from behind only through a surface that transmits. A
        // collision's stack is the phase function; the surface's
        // evaluation runs for it too, of an empty stack, and is left.
        let below = dot(draw.direction, surf.geometric) <= 0.0;
        if (!medium && below && !transmissive) {
            continue;
        }
        let wi = to_local(f, draw.direction);
        if (!medium && (wi.z == 0.0 || (wi.z < 0.0 && !transmissive))) {
            continue;
        }
        let value = openpbr_eval_selective(bsdf, wo, wi, diffuse, specular);
        if (!medium && all(value == vec3<f32>(0.0))) {
            continue;
        }
        // The stack's density, and its seeing lobes', in one pass.
        let densities = openpbr_densities(bsdf, wo, wi, SEEN_UP_TO, PASSED_FROM);
        let phase = md_phase(md_dot(wo, draw.direction), here.g);
        let bsdf_pdf = select(densities.x, 0.0, draw.is_delta);
        var visibility = vec3<f32>(1.0);
        var bent = false;
        if (frame.shadows != 0u && shadow) {
            // Just short of a lamp; past everything for the sky or a sun;
            // out of the side the light is on. From a collision, from the
            // point itself, through the medium.
            var reach = frame.shadow_far;
            if (!draw.infinite) {
                reach = draw.distance * (1.0 - 1e-3);
            }
            var ray = ShadowRay(surf.position, draw.direction, reach);
            if (!medium) {
                let leaving = select(surf.geometric, -surf.geometric, below);
                var aim = surf.position + draw.direction * reach;
                var aim_normal = -draw.direction;
                if (glow) {
                    aim = glow_point;
                    aim_normal = glow_normal;
                }
                ray = shadow_ray(offset_at(surf.position, leaving, surf.magnitude), leaving, aim, aim_normal);
            }
            // The light sample's own walk (walk_ray's second place, see
            // there): walked here, the stack still at hand. The sky's passes
            // the floors the bounces pass where it sees past them (a kernel
            // part with the floor's: with no floor nothing answers
            // MASK_FLOOR, and either mask is the same).
            var walk = shadow_walk_of(frame.shadow_glass, ray, medium, instance, here.sigma, blockers);
            walk.past_floors = FEATURE_FLOOR && sky && !medium && (frame.lamp_flags & LAMPS_SKY_PAST_FLOORS) != 0u;
            let v = walk_ray(walk);
            if (!medium && all(v.through == vec3<f32>(0.0))) {
                continue;
            }
            visibility = v.through;
            bent = v.bent;
        }
        if (medium) {
            var lit: vec3<f32>;
            if (sky) {
                lit = (draw.radiance * visibility) * cy_div(cy_exact(phase * mis_power(draw.pdf, phase)), draw.pdf);
            } else {
                lit = (draw.radiance * visibility) * cy_div(phase, select(draw.pdf * p_select, p_select, draw.is_delta));
            }
            direct += clamp_light(here.throughput * lit, here.ceiling);
            continue;
        }
        var scale: vec3<f32>;
        if (!(source == 1u)) {
            // The sky, or a glowing triangle, which the bounce can meet too.
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
        if (FEATURE_SPECTRAL) {
            let lit = scale * visibility;
            if (source == 0u) {
                parts.values[0] = value;
                parts.scales[0] = lit;
            } else if (source == 1u) {
                parts.values[1] = value;
                parts.scales[1] = lit;
            } else {
                parts.values[2] = value;
                parts.scales[2] = lit;
            }
            if (wi.z < 0.0) {
                parts.through |= 1u << source;
            }
        }
    }
    parts.total = direct;
    return parts;
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
// the point), and its handedness, -1 or 1; w 0 where the mesh has none.
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
    let hand = u * t0.w + v * t1.w + w * t2.w;
    return vec4<f32>(rows_vector(inst.object_to_world, t), select(1.0, -1.0, hand < 0.0));
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

// What the snapshot measured at a hit's corners, at a point of the part's
// space (SceneSnapshot's curvature_at, occlusion_at, closeness_at and
// vertex_color_at): the barycentrics worked out again from the point as
// `barycentrics` works them out, each value the corners' weighed, the
// curvature by the part's scale, the colour brought from linear sRGB into
// the rendering space; nought, nought, nought and white where the mesh has
// none (or the corners are degenerate). Every product an add reads rounded
// as Rust rounds it.
struct MeshMeasures {
    curvature: f32,
    occlusion: f32,
    closeness: f32,
    colour: vec3<f32>,
};

fn exact_dot(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return cy_exact(cy_exact(a.x * b.x) + cy_exact(a.y * b.y)) + cy_exact(a.z * b.z);
}

fn weighed(b: vec3<f32>, x: f32, y: f32, z: f32) -> f32 {
    return cy_exact(cy_exact(x * b.x) + cy_exact(y * b.y)) + cy_exact(z * b.z);
}

fn mesh_measures(inst: GpuInstance, mesh: GpuMesh, c: vec3<u32>, object: vec3<f32>) -> MeshMeasures {
    var m = MeshMeasures(0.0, 0.0, 0.0, vec3<f32>(1.0));
    if (!FEATURE_ATTRIBUTES || mesh.attribute_base == NO_DATA) {
        return m;
    }
    let p0 = scene_position(mesh.base_vertex + c.x);
    let e1 = scene_position(mesh.base_vertex + c.y) - p0;
    let e2 = scene_position(mesh.base_vertex + c.z) - p0;
    let d = object - p0;
    let d11 = exact_dot(e1, e1);
    let d12 = exact_dot(e1, e2);
    let d22 = exact_dot(e2, e2);
    let d31 = exact_dot(d, e1);
    let d32 = exact_dot(d, e2);
    let det = cy_exact(d11 * d22) - cy_exact(d12 * d12);
    if (abs(det) < 1e-30) {
        return m;
    }
    let v = tex_div(cy_exact(d22 * d31) - cy_exact(d12 * d32), det);
    let w = tex_div(cy_exact(d11 * d32) - cy_exact(d12 * d31), det);
    let b = vec3<f32>(1.0 - v - w, v, w);
    let at = 2u * mesh.attribute_base;
    let a0 = scene_attributes[at + 2u * c.x];
    let a1 = scene_attributes[at + 2u * c.y];
    let a2 = scene_attributes[at + 2u * c.z];
    let flags = mesh.attribute_flags;
    if ((flags & HAS_CURVATURE) != 0u) {
        m.curvature = weighed(b, a0.x, a1.x, a2.x) * inst.inv_length_scale;
    }
    if ((flags & HAS_OCCLUSION) != 0u) {
        m.occlusion = weighed(b, a0.y, a1.y, a2.y);
    }
    if ((flags & HAS_CLOSENESS) != 0u) {
        m.closeness = weighed(b, a0.z, a1.z, a2.z);
    }
    if ((flags & HAS_COLORS) != 0u) {
        let c0 = scene_attributes[at + 2u * c.x + 1u];
        let c1 = scene_attributes[at + 2u * c.y + 1u];
        let c2 = scene_attributes[at + 2u * c.z + 1u];
        m.colour = tex_acescg(vec3<f32>(weighed(b, c0.x, c1.x, c2.x), weighed(b, c0.y, c1.y, c2.y), weighed(b, c0.z, c1.z, c2.z)));
    }
    return m;
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

// Where bevel_normal stands while path() walks its probes: the sample it is
// at and whether that sample's three draws are taken - its axis, its way
// across the disc and how far, and the height either side of it - the probe
// of the two it is at, whether it waits on that probe's walk, and the
// normals lent so far, weighed.
struct BevelAt {
    i: u32,
    k: u32,
    drawn: bool,
    pending: bool,
    axis: vec3<f32>,
    across: vec3<f32>,
    r: f32,
    height: f32,
    sum: vec3<f32>,
};

// What bevel_normal hands back: a probe to walk, or the rounded normal.
struct BevelStep {
    wants: bool,
    walk: Walk,
    normal: vec3<f32>,
};

// Rounded edges (bevel.rs's rounded_normal, ic:1680-1691), as Cycles' Bevel
// node rounds them: the shading normal `normal` at `position` on the part
// in slot `slot` blended with the normals of the faces of the same part
// within `radius`, from `samples` samples. Each takes a point on a disc of
// that radius across one of the frame's three axes - the normal half the
// time, either tangent a quarter, on three draws of the open stream - and
// casts a short ray through the disc from each side (WALK_FIRST: the
// closest surface of any part, from the start to twice the height); what it
// meets of the same part, near enough and not leaning the other way, lends
// its normal, weighed down with its distance. A fresh BevelAt with its
// `sum` the normal times 1e-3 starts it; each probe is path()'s walk,
// `found` once walked.
fn bevel_normal_step(slot: u32, position: vec3<f32>, normal: vec3<f32>, radius: f32, samples: u32, s: ptr<function, Sampler>, at: ptr<function, BevelAt>, found: WalkEnd) -> BevelStep {
    var out: BevelStep;
    out.wants = false;
    let sigma = radius * 0.5;
    let count = max(samples, 1u);
    if ((*at).pending) {
        (*at).pending = false;
        let near = found.hit;
        if (!(near.kind == RAY_QUERY_INTERSECTION_NONE || near.instance_custom_data != slot)) {
            // The far side of a thin part leans the other way: not an edge.
            let seen = surface_of(near);
            if (!(cy_dot3(seen.shading, normal) < -0.1)) {
                let d = seen.position - position;
                let distance = cy_sqrt(cy_dot3(d, d));
                let weight = exp(cy_div(-(distance * distance), 2.0 * sigma * sigma));
                (*at).sum += cy_exact3(seen.shading * weight);
            }
        }
        (*at).k += 1u;
    }
    if ((*at).k >= 2u) {
        (*at).i += 1u;
        (*at).k = 0u;
        (*at).drawn = false;
    }
    if ((*at).i >= count) {
        out.normal = prog_normalized((*at).sum, normal);
        return out;
    }
    if (!(*at).drawn) {
        let f = frame_of(normal);
        let pick = sampler_next_unstructured(s);
        var axis = f.n;
        var across = f.t;
        var other = f.b;
        if (pick >= 0.75) {
            axis = f.b;
            across = f.n;
            other = f.t;
        } else if (pick >= 0.5) {
            axis = f.t;
            across = f.b;
            other = f.n;
        }
        let r = radius * cy_sqrt(sampler_next_unstructured(s));
        let phi = TAU * sampler_next_unstructured(s);
        let height = max(cy_sqrt(max(cy_exact(radius * radius) - cy_exact(r * r), 0.0)), radius * 1e-3);
        let turn = cy_sincos(phi);
        (*at).axis = axis;
        (*at).across = cy_exact3(across * turn.y) + cy_exact3(other * turn.x);
        (*at).r = r;
        (*at).height = height;
        (*at).drawn = true;
    }
    let side = select(1.0, -1.0, (*at).k == 1u);
    let axis = (*at).axis;
    let height = (*at).height;
    let offset = (*at).across * (*at).r;
    let start = (position + offset) + axis * (height * side);
    (*at).pending = true;
    out.wants = true;
    out.walk = walk_new(WALK_FIRST, start, axis * -side, 2.0 * height);
    return out;
}

// bevel_normal_step with its probes walked at once, for the test kernels
// (tests/bevel.rs); the transport walks them at path()'s one place.
fn bevel_normal(slot: u32, position: vec3<f32>, normal: vec3<f32>, radius: f32, samples: u32, s: ptr<function, Sampler>) -> vec3<f32> {
    var at: BevelAt;
    at.sum = normal * 1e-3;
    var found: WalkEnd;
    var rounded: vec3<f32>;
    loop {
        let step = bevel_normal_step(slot, position, normal, radius, samples, s, &at, found);
        if (!step.wants) {
            rounded = step.normal;
            break;
        }
        found = walk_ray(step.walk);
    }
    return rounded;
}

// The material at `hit`, seen along `direction` by a ray of kind `ray`
// whose footprint widens by `spread` (ic:1626-1710): the shading point its
// program reads (SceneSnapshot::shading_point_along), and prog_shade's
// program, write-back and bump at it, the bump bending `rounded` - the
// shading normal its edges round to (bevel_normal), the point's own where
// they do not.
fn material_at(hit: RayIntersection, surf: Surface, direction: vec3<f32>, ray: u32, spread: f32, rounded: vec3<f32>) -> ProgShaded {
    let inst = scene_instances[hit.instance_custom_data];
    let p = scene_programs[inst.material];
    let base = scene_materials[inst.material];
    var s: ProgSurface;
    s.normal = rounded;
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
    let measures = mesh_measures(inst, mesh, c, pt.object);
    pt.curvature = measures.curvature;
    pt.occlusion = measures.occlusion;
    pt.closeness = measures.closeness;
    pt.vertex_color = measures.colour;
    // No ray differentials: the footprint widens with the distance alone.
    let world = hit.t * spread;
    s.footprint = world * inst.inv_length_scale;
    pt.footprint = s.footprint;
    let derivatives = uv_derivatives(inst, mesh, c);
    // The same footprint in UV: its length over how far the surface moves
    // per unit of u, and of v.
    if (derivatives.ok) {
        let lu = cy_sqrt(cy_dot3(derivatives.dpdu, derivatives.dpdu));
        let lv = cy_sqrt(cy_dot3(derivatives.dpdv, derivatives.dpdv));
        if (lu > 1e-12 && lv > 1e-12) {
            pt.uv_footprint = vec2<f32>(tex_div(world, lu), tex_div(world, lv));
            pt.has_uv_footprint = 1u;
        }
    }
    s.has_derivatives = derivatives.ok;
    s.dpdu = derivatives.dpdu;
    s.dpdv = derivatives.dpdv;
    s.object_to_world = inst.object_to_world;
    s.world_to_object = inst.world_to_object;
    if (FEATURE_TEXTURES && (p.flags & PROGRAM_NORMAL_UV) != 0u) {
        s.mesh_tangent = mesh_tangent(inst, mesh, c, pt.object);
    }
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
    if (FEATURE_SPECTRAL && spectral_on) {
        let tp4 = (*st).tp4;
        let q4 = clamp(max(max(max(tp4.x, tp4.y), tp4.z), tp4.w) * (*st).eta_scale, 0.0, 0.95);
        if (sampler_russian_roulette(s) >= q4) {
            return false;
        }
        (*st).tp4 = tp4 * (1.0 / q4);
        (*st).roulette = cy_div((*st).roulette, q4);
        return true;
    }
    let throughput = (*st).throughput;
    let q = clamp(max(max(throughput.x, throughput.y), throughput.z) * (*st).eta_scale, 0.0, 0.95);
    if (sampler_russian_roulette(s) >= q) {
        return false;
    }
    (*st).throughput = throughput * (1.0 / q);
    (*st).roulette = cy_div((*st).roulette, q);
    return true;
}

// integrator.rs's glass_pass, its number: what a crossing's `weight` scales
// the lobe's tint `root` (the square root of the glass's colour) by, the
// same in every channel the root has; nought where the weight is not that
// root scaled - glass under a coloured coat - and the weight is read as it
// is.
fn glass_pass_scale(root: vec3<f32>, weight: vec3<f32>) -> f32 {
    let has = root > vec3<f32>(1e-4);
    let k = vec3<f32>(cy_div(weight.x, select(1.0, root.x, has.x)), cy_div(weight.y, select(1.0, root.y, has.y)), cy_div(weight.z, select(1.0, root.z, has.z)));
    // The first channel the root has.
    let scale = select(select(k.z, k.y, has.y), k.x, has.x);
    if (!any(has) || !(is_finite(scale) && scale > 0.0)) {
        return 0.0;
    }
    let apart = abs(k - vec3<f32>(scale)) > vec3<f32>(1e-3 * scale);
    let stray = weight > vec3<f32>(1e-6);
    if (any(select(stray, apart, has))) {
        return 0.0;
    }
    return scale;
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
// roulette (ic:1862-2562), off the material `shaded` (material_at).
fn shade_vertex(st: ptr<function, PathState>, s: ptr<function, Sampler>, hit: RayIntersection, surf: Surface, shaded: ProgShaded, split: ptr<function, SplitOff>, light: ptr<function, LightSampleAt>) -> u32 {
    let bounce = (*st).bounce;
    // Passes through glass spend no bounce (ic:1944).
    if (bounce - (*st).glass_crossings >= (*st).max_bounces) {
        return SHADE_END;
    }
    // The frame: along the anisotropy's tangent where the material is
    // anisotropic (ic:1866-1885).
    var material = shaded.material;
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
    // Dispersion (ic:1912-1946): glass with an Abbe number bends as light of
    // one wavelength does - drawn at the first such glass a path meets (a
    // draw of the open stream), the path weighed by the colour it stands
    // for, and kept from there on, entering and leaving alike, so a medium
    // is left by the index it was entered with.
    // A path carrying wavelengths bends by its hero's; it keeps all four
    // off the surface, and only the hero goes on through it (below).
    let disperses = FEATURE_DISPERSION && material.transmission_color.w > 0.0 && material.film.w > 0.0;
    if (disperses) {
        var lambda = (*st).wavelength;
        if (FEATURE_SPECTRAL && spectral_on) {
            lambda = spectral_lambda.x;
        } else if (lambda <= 0.0) {
            (*st).wavelength = dispersion_sample(sampler_next_unstructured(s));
            (*st).throughput = (*st).throughput * dispersion_weight((*st).wavelength);
            lambda = (*st).wavelength;
        }
        material.specular_ior = dispersion_ior_at(material.specular_ior, material.film.w, lambda);
    }
    // Which side of the surface the path is on, read from the media it has
    // crossed into rather than from the winding, and the index between the
    // medium here and the one beyond: going from a denser medium into a
    // lighter one is the interface seen from inside (ic:1947-1970). The
    // stack's interface and transmission read that index.
    let boundary = FEATURE_GLASS && (material.glass & GLASS_BOUNDARY) != 0u;
    // A walk that reaches its own part's surface from inside leaves through
    // it, spread over the hemisphere outside (ic:1975). A walk is keyed by
    // the part, a medium's boundary by its solid (GpuInstance::solid).
    let exiting_walk = FEATURE_SUBSURFACE && media_walking_in((*st).media, inst.instance);
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
    // Metallic paint's flakes (ic:1668-1678): which flake this point lies
    // on, if any, and how it leans, in the part's object space. No program
    // on the card drives them, so the record's weight is the point's.
    if (FEATURE_FLAKES && lobes.flake_color.w > 1e-4) {
        let at = flake_at(rows_point(inst.world_to_object, surf.position), inst.scale, lobes.flake.x, lobes.flake_color.w, lobes.flake.y);
        lobes.flake.w = at.w;
        lobes.flake_normal = vec4<f32>(at.xyz, 0.0);
    }
    if (exiting_walk) {
        lobes = walk_exit_material(lobes);
    }
    // Clear glass the last light sample was taken through, met on the way
    // on by a bounce that passes it - and what that bounce left by, weighed
    // as the lobes that pass glass that bends light are; glass met as glass
    // by a seeing bounce (ic:1974-1991).
    let pane = FEATURE_PANES && (material.glass & GLASS_PANE) != 0u && !exiting_walk;
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
    // None where the lights are not sampled directly (direct_lighting off).
    let samples_lights = !pure_delta(bsdf) && !passing && (frame.lamp_flags & LAMPS_UNSAMPLED) == 0u;
    // Where the glowing parts were last sampled from: here, if they are
    // sampled here; kept through glass being passed (ic:2057, 2117).
    if (!passing) {
        (*st).receiver = false;
    }
    if (samples_lights) {
        // Lit from one side, it draws only the sky it can see: a surface
        // that only reflects; one whose only way through is a walk's
        // doorway, behind which is the part itself; a walk's way out,
        // whose light comes from outside alone (ic:2069-2081).
        let doorway_only = (material.glass & GLASS_THIN) == 0u && material.transmission_color.w <= 0.0;
        if (exiting_walk) {
            (*st).sky_facing = -surf.geometric;
        } else {
            (*st).sky_facing = select(vec3<f32>(0.0), surf.geometric, !transmissive || doorway_only);
        }
        if (FEATURE_EMITTERS && (frame.glass_flags & SCENE_EMITTERS) != 0u) {
            (*st).receiver = true;
            (*st).receiver_position = surf.position;
            (*st).receiver_normal = surf.geometric;
            (*st).receiver_two_sided = transmissive;
        }
        // Taken by path's one call of direct_light as the vertex is left,
        // nothing between reading the radiance or meeting a crossing; its
        // draws here, before the bounce's, as the processor draws them: the
        // sky's where it lights, the lamp's choice from this point and the
        // lamp's where it lights this part, the glowing part's.
        *light = LightSampleAt(true, false, bsdf, f, wo, surf, inst.instance, transmissive, (*st).sky_facing, (*st).throughput, light_ceiling((*st).specular_chain, bounce, (*st).roulette), vec3<f32>(0.0), 0.0, (*st).tp4, (*st).alone, disperses, vec2<f32>(0.0), LampChoice(false, 0u, 0.0), vec2<f32>(0.0), 0.0, vec2<f32>(0.0));
        if (frame.env_intensity > 0.0) {
            (*light).u_sky = sampler_light_sample(s);
        }
        if (FEATURE_LAMPS && frame.light_count > 0u) {
            let choice = lamps_select_at(frame.light_count, frame.light_hierarchy != 0u, surf.position, surf.geometric, transmissive, sampler_light_select(s));
            (*light).choice = choice;
            if (choice.ok && lamp_lights(scene_lights[choice.index], inst.instance)) {
                (*light).u_lamp = sampler_light_sample(s);
            }
        }
        if ((frame.glass_flags & SCENE_EMITTERS) != 0u) {
            (*light).u_glow_pick = sampler_light_select(s);
            (*light).u_glow = sampler_light_sample(s);
        }
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
    // Carrying wavelengths, only while they all share one weight, which the
    // reflection's own wavelengths are filtered by (ic:2277-2286).
    let shares = !(FEATURE_SPECTRAL && spectral_on) || all((*st).tp4 == vec4<f32>((*st).tp4.x));
    if (FEATURE_PANES && shares && bounce == 0u && !(*st).reflected && !(*st).in_floor && !exiting_walk && (material.glass & (GLASS_PANE | GLASS_PLAIN)) == (GLASS_PANE | GLASS_PLAIN)) {
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
            var carried = (*st).throughput;
            if (FEATURE_SPECTRAL && spectral_on) {
                carried = vec3<f32>((*st).tp4.x);
            }
            *split = SplitOff(offset_at(surf.position, surf.geometric, surf.magnitude), mirrored, share, carried, false);
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
    var tp4 = (*st).tp4;
    if (FEATURE_SPECTRAL && spectral_on) {
        // A pass through clear glass of a colour takes the glass's crossing
        // tint, scaled by the number its weight scales the lobe's tint by
        // (#187, ic:2438-2442's glass_pass): two crossings are the colour,
        // where the weight's own spectrum read a garnet too red. Anything
        // else is read by its weight; one look-up either way.
        let weight = sample.f * (abs(sample.wi.z) / sample.pdf);
        var read = weight;
        var by = 1.0;
        if ((material.glass & GLASS_TINTED) != 0u && sample.is_transmission && (splits || chosen.slot != LOBE_SUBSURFACE)) {
            let k = glass_pass_scale(material.transmission_tint.xyz, weight);
            if (k > 0.0) {
                read = crossing_tint_of(material);
                by = k;
            }
        }
        tp4 = tp4 * cy_exact4(spectral_of(read, false) * by);
        // Through a dispersive surface each wavelength leaves its own way:
        // the hero goes on for all four (ic:2356-2363).
        if (disperses && sample.is_transmission && !(*st).alone) {
            tp4 = spectral_hero_alone(tp4);
            (*st).alone = true;
        }
    }
    // Through the surface: counted against the transmissions, the relative
    // index squared kept for Russian roulette, and a medium's boundary
    // entered or left, recorded with the surface's own index (ic:2373-2399).
    if ((FEATURE_GLASS || FEATURE_SUBSURFACE) && sample.is_transmission) {
        // A pass through glass - refracted, smooth or rough, through a thin
        // wall, or the split's - is no bounce (ic:2453-2455); the way into a
        // walk and a thin sheet's diffuse light through it (the subsurface
        // slot) scatter as a bounce does.
        if (splits || chosen.slot != LOBE_SUBSURFACE) {
            (*st).glass_crossings += 1u;
        }
        (*st).transmission_bounces += 1u;
        if ((*st).transmission_bounces > frame.max_transmission_bounces) {
            return shade_ends(split, splitting);
        }
        (*st).eta_scale *= sample.eta * sample.eta;
        // Through the subsurface lobe: a walk begins under the surface (of
        // the shaded material), guided back towards this point and out
        // through this surface; or the one the path was on ends
        // (ic:2388-2395). Else a boundary's medium entered or left. One
        // crossing of the stack for both.
        let walking = FEATURE_SUBSURFACE && chosen.slot == LOBE_SUBSURFACE && bsdf.subsurface_walk;
        if (walking || boundary) {
            // The walk's medium's albedo: the shaded material's.
            let albedo = select(vec3<f32>(0.0), material.subsurface_albedo.xyz, walking);
            // Through a local: naga's SPIR-V writer cannot pass a pointer
            // to a member.
            var media = (*st).media;
            media_cross_walk(&media, select(inst.solid, inst.instance, walking), material.medium_priority, material.specular_ior, inst.material, walking, albedo);
            (*st).media = media;
            if (walking) {
                (*st).guide = md_no_guide();
                if (media_walking_in(media, inst.instance)) {
                    (*st).guide = md_guide_new(inst.instance, surf.position, exact_geometric(hit, (*st).direction), albedo, material.subsurface_extinction.w);
                }
            }
        }
    }
    if (FEATURE_SPECTRAL && spectral_on) {
        if (!(is_finite(tp4.x) && is_finite(tp4.y) && is_finite(tp4.z) && is_finite(tp4.w)) || all(tp4 == vec4<f32>(0.0))) {
            return shade_ends(split, splitting);
        }
        (*st).tp4 = tp4;
    } else {
        if (!(is_finite(throughput.x) && is_finite(throughput.y) && is_finite(throughput.z)) || all(throughput == vec3<f32>(0.0))) {
            return shade_ends(split, splitting);
        }
        (*st).throughput = throughput;
    }
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
        // Turned back off glass being passed, the path goes where no light
        // sample of the glowing parts looked (ic:2439).
        if (passing) {
            (*st).receiver = false;
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

/// ---- The shadow catcher -------------------------------------------------

// blocked_towards with no glass let by, as ambient occlusion asks it: one
// shadow ray from the point to `reach` along `direction` (WALK_SHADOW, no
// glass let by, no lamp: cutout_transmittance where cutouts are, else one
// any-hit query, Intersector::occluded), and 1 less the luminance of what
// gets through (blocked_of) - 0 or 1, or through cutouts what their
// opacities leave, alike in every lane.
fn blocked_walk(position: vec3<f32>, magnitude: f32, normal: vec3<f32>, direction: vec3<f32>, reach: f32) -> Walk {
    let aim = position + direction * reach;
    let from_point = offset_at(position, normal, magnitude);
    return shadow_through_walk(GLASS_SOLID, from_point, normal, aim, -direction, NO_LIGHT);
}

fn blocked_of(found: WalkEnd) -> f32 {
    return 1.0 - clamp(luminance(vec3<f32>(found.through.x)), 0.0, 1.0);
}

// occlusion_at: ambient occlusion within `radius`, one cosine-weighted
// direction drawn from the light-sample dimensions of whatever bounce the
// sampler is on; the occlusion blocked_of the walk.
fn occlusion_walk(position: vec3<f32>, magnitude: f32, normal: vec3<f32>, radius: f32, s: ptr<function, Sampler>) -> Walk {
    let direction = occlusion_direction(normal, s);
    // A finite distance: stopped a thousandth short of it.
    return blocked_walk(position, magnitude, normal, direction, max(radius, 1e-3) * (1.0 - 1e-3));
}

// ic:3065: how many directions shadow_at draws before it keeps the last.
const MAX_SHADOW_DRAWS: u32 = 32u;

// What shadow_at hands back: the shadow ray to walk, the share of the light
// blocked then 1 less the luminance of what gets through it; or that share
// with no shadow ray.
struct FloorShadow {
    walks: bool,
    walk: Walk,
    share: f32,
    // open_irradiance (#171), where asked for.
    open: f32,
};

// shadow_at (ic:3087-3177): the share of the light onto this point of the
// floor `instance` the scene blocks, one direction drawn in proportion to
// the light it would bring, from the sky and every lamp together - each by
// its bound onto the point (Light::irradiance_bound; a lamp whose light
// links leave the floor out none), the sky chosen where the pick falls
// short of its own, else the lamp it falls in - kept or not against that
// bound: a floor facing straight up keeps the sky's first draw; anything
// else keeps a draw by its cosine, the draws after the first on the
// dimensions of the bounces after. The shadow ray passes the clear glass
// the configuration's shadow rays pass, as blocked_towards with the
// configuration's glass does (ic:3141), a lamp's by its shadow links, and a
// lamp that casts no shadow blocks nothing.
//
// And, where `opens` asks (#171), open_irradiance (ic:3105-3121): the light,
// as luminance, the environment and the lamps bring onto the point with
// nothing in the way, which the product's light on the floor is measured
// against - the sky's bound, and each lamp's light from its middle (a light
// sample at (0.5, 0.5)), through the draws' one lamp_sample, ahead of them;
// with `draws` false (a floor that takes its shadow as occlusion) that
// alone, drawing nothing.
fn shadow_at(position: vec3<f32>, magnitude: f32, normal: vec3<f32>, bounce: u32, s: ptr<function, Sampler>, instance: u32, draws: bool, opens: bool) -> FloorShadow {
    var none: FloorShadow;
    none.walks = false;
    none.share = 0.0;
    none.open = 0.0;
    let shadows = draws && frame.shadows != 0u;
    if (!shadows && !(FEATURE_PRODUCT_LIGHT && opens)) {
        return none;
    }
    // With a dome of a size, the sky's light onto a point grows by up to
    // its spread bound there (irradiance_bound).
    let finite = dome_finite();
    var total = env_irradiance_bound(normal);
    var spread = 1.0;
    if (finite) {
        spread = dome_spread_bound(position);
        total *= spread;
    }
    let sky = total;
    var open = sky;
    var lamps = 0u;
    if (FEATURE_LAMPS) {
        lamps = frame.light_count;
        if (shadows) {
            for (var i = 0u; i < lamps; i++) {
                total += floor_lamp_bound(i, position, normal, instance);
            }
        }
    }
    let lit = select(0u, lamps, FEATURE_PRODUCT_LIGHT && opens);
    let count = lit + select(0u, MAX_SHADOW_DRAWS, shadows && total > 0.0);
    for (var k = 0u; k < count; k++) {
        let opening = k < lit;
        let draw = k - lit;
        var pick = 0.0;
        var u = vec2<f32>(0.5);
        var chance = 0.0;
        var last = false;
        if (!opening) {
            sampler_begin_bounce(s, bounce + draw);
            // Drawn in the processor's order, the open stream's past the
            // Sobol' limit.
            pick = sampler_light_select(s);
            u = sampler_light_sample(s);
            chance = sampler_bsdf_lobe(s);
            last = draw + 1u == MAX_SHADOW_DRAWS;
        }
        var keep: f32;
        var towards: vec3<f32>;
        var reach = frame.shadow_far;
        var blockers = NO_LIGHT;
        var casts = true;
        var along = pick * total;
        if (!opening && (lamps == 0u || along < sky)) {
            let sample = env_sample_irradiance(normal, u);
            if (!sample.ok) {
                continue;
            }
            // Seen from here rather than from the dome's centre, a patch of
            // a dome of a size spans 1 / jacobian of the solid angle, and the
            // shadow ray stops at the dome (sample_irradiance).
            keep = sample.keep;
            towards = sample.direction;
            if (finite) {
                let seen = dome_from_centre(position, sample.direction);
                if (!seen.ok) {
                    continue;
                }
                keep = min(cy_div(max(dot(normal, seen.direction), 0.0), cy_exact(seen.jacobian * spread)), 1.0);
                towards = seen.direction;
                reach = seen.distance * (1.0 - 1e-3);
            }
        } else {
            // The lamp the pick falls in, rounding past the end the last;
            // or, for the open light, each in turn.
            var chosen = k;
            var bound = 1.0;
            if (!opening) {
                along -= sky;
                chosen = lamps - 1u;
                bound = 0.0;
                for (var i = 0u; i < lamps; i++) {
                    bound = floor_lamp_bound(i, position, normal, instance);
                    if (along < bound) {
                        chosen = i;
                        break;
                    }
                    along -= bound;
                }
                if (bound <= 0.0) {
                    continue;
                }
            }
            let light = scene_lights[chosen];
            let sample = lamp_sample(light, position, u);
            if (!sample.ok) {
                continue;
            }
            let density = select(sample.pdf, 1.0, sample.is_delta);
            if (!(density > 0.0)) {
                continue;
            }
            if (opening) {
                open += cy_div(cy_exact(luminance(sample.radiance) * max(dot(sample.direction, normal), 0.0)), density);
                continue;
            }
            keep = luminance(sample.radiance) * max(dot(sample.direction, normal), 0.0) / density / bound;
            towards = sample.direction;
            if (!sample.infinite) {
                reach = sample.distance * (1.0 - 1e-3);
            }
            casts = light_has(light, LF_SHADOW);
            blockers = chosen;
        }
        if (chance < keep || (last && keep > 0.0)) {
            if (!casts) {
                none.open = open;
                return none;
            }
            var shadow: FloorShadow;
            shadow.walks = true;
            shadow.walk = shadow_through_walk(frame.shadow_glass, offset_at(position, normal, magnitude), normal, position + towards * reach, -towards, blockers);
            shadow.share = 0.0;
            shadow.open = open;
            return shadow;
        }
    }
    none.open = open;
    return none;
}

// The share of the light shadow_at's shadow ray blocks, once walked.
fn shadow_at_of(found: WalkEnd) -> f32 {
    return 1.0 - clamp(luminance(found.through), 0.0, 1.0);
}

// A lamp's bound onto the point of floor `instance` (shadow_at's bounds): its
// irradiance_bound, none where its light links leave the floor out.
fn floor_lamp_bound(i: u32, position: vec3<f32>, normal: vec3<f32>, instance: u32) -> f32 {
    let light = scene_lights[i];
    if (!lamp_lights(light, instance)) {
        return 0.0;
    }
    return light_irradiance_bound(light, position, normal);
}

// What a catcher a path meets other than as the camera would see it does
// (ic:1472-1519): ends the path glowing, lets it through, or is shaded as
// its material.
const MET_SHADE: u32 = 0u;
const MET_PASSED: u32 = 1u;
const MET_END: u32 = 2u;

// `open`: what of a glowing floor's bounce its falloff's occlusion leaves,
// walked before (1 without a falloff).
fn floor_met(st: ptr<function, PathState>, c: GpuCatcher, raw: Surface, open: f32) -> u32 {
    let direction = (*st).direction;
    // A floor with a bounce glows for everything but the camera, darker
    // toward the contact over its falloff.
    if (c.bounce.w != 0.0) {
        if (FEATURE_SPECTRAL && spectral_on) {
            (*st).rad4 += cy_exact4((*st).tp4 * spectral_of(c.bounce.xyz, true) * open);
        } else {
            (*st).radiance += (*st).throughput * c.bounce.xyz * open;
        }
        return MET_END;
    }
    // Ground illumination shades the floor as its material.
    if (frame.ground_illumination != 0u) {
        return MET_SHADE;
    }
    // Else straight on through it, to what lies below the horizon. The
    // lamps' and the glowing parts' light samples did not see past it, so
    // what of them is found beyond takes full weight, as past a mirror; the
    // sky's did where it sees past such floors (sky_past_floors), and the
    // sky beyond keeps the weight the bounce had. Elsewhere the sky too
    // takes full weight.
    if ((*st).crossings >= MAX_CUTOUT_CROSSINGS) {
        return MET_END;
    }
    (*st).crossings += 1u;
    let g = select(raw.geometric, -raw.geometric, dot(direction, raw.geometric) > 0.0);
    (*st).origin = offset_at(raw.position, -g, raw.magnitude);
    if ((frame.lamp_flags & LAMPS_SKY_PAST_FLOORS) != 0u) {
        (*st).receiver = false;
    } else {
        (*st).prev_pdf = PDF_INFINITE;
    }
    (*st).through_glass = GLASS_SOLID;
    (*st).seen.valid = false;
    return MET_PASSED;
}

// What the floor shows and how much of the backdrop it hides.
struct Caught {
    radiance: vec3<f32>,
    coverage: f32,
};

// floor_reflection up to its path (ic:2982-3026): the mirror ray off a GGX
// facet of the floor's roughness, or off its normal; not ok where it does
// not leave the floor. By distance, how near the product is - the walk of
// nearness_at or occlusion_at, made before its path - and its origin is
// then offset_at the floor's point along `normal`.
struct MirrorRay {
    ok: bool,
    direction: vec3<f32>,
    // How squarely it leaves the floor: the reflection's cosine to it.
    rise: f32,
};

fn mirror_ray(c: GpuCatcher, normal: vec3<f32>, view: vec3<f32>, s: ptr<function, Sampler>) -> MirrorRay {
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
    return MirrorRay(true, reflected, rise);
}

// A floor the camera sees, held while its reflection's path is traced: the
// path that met it - what it has gathered and carries, and its bounce - and
// the floor - what it stands in front of, its shadow, the catcher and the
// way it is seen - with the mirror ray's rise and nearness.
struct FloorSeen {
    radiance: vec3<f32>,
    throughput: vec3<f32>,
    // The same at four wavelengths, carrying them, and whether the hero
    // went on alone.
    rad4: vec4<f32>,
    tp4: vec4<f32>,
    alone: bool,
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

// cy_exact for a pair: each product rounded as Rust rounds it.
fn cy_exact2(v: vec2<f32>) -> vec2<f32> {
    return bitcast<vec2<f32>>(bitcast<vec2<u32>>(v) | vec2<u32>(cy_zero));
}

// Camera::sample_aperture (camera.rs): a point on the lens's opening, of
// radius 1, for the lens draw `u` - sample_polygon_aperture for one of
// blades (a wedge by u.x, then a point in its triangle, folded),
// sample_uniform_disk_concentric for a round one (sampling.rs).
fn aperture_point(u: vec2<f32>) -> vec2<f32> {
    if (FEATURE_BLADES && (frame.camera_flags & CAMERA_BLADED) != 0u) {
        let blades = f32(frame.aperture_blades);
        let along = cy_exact(u.x * blades);
        let wedge = min(floor(along), blades - 1.0);
        var s = clamp(along - wedge, 0.0, 1.0);
        var t = u.y;
        let a0 = cy_div(cy_exact(CY_TAU * wedge), blades) + frame.aperture_rotation;
        let a1 = cy_div(cy_exact(CY_TAU * (wedge + 1.0)), blades) + frame.aperture_rotation;
        let v0 = cy_sincos(a0).yx;
        let v1 = cy_sincos(a1).yx;
        if (s + t > 1.0) {
            s = 1.0 - s;
            t = 1.0 - t;
        }
        return cy_exact2(v0 * s) + cy_exact2(v1 * t);
    }
    let o = vec2<f32>(2.0 * u.x - 1.0, 2.0 * u.y - 1.0);
    if (o.x == 0.0 && o.y == 0.0) {
        return vec2<f32>(0.0);
    }
    var r: f32;
    var theta: f32;
    if (abs(o.x) > abs(o.y)) {
        r = o.x;
        theta = cy_exact((CY_PI / 4.0) * cy_div(o.y, o.x));
    } else {
        r = o.y;
        theta = (CY_PI / 2.0) - cy_exact((CY_PI / 4.0) * cy_div(o.x, o.y));
    }
    let sc = cy_sincos(theta);
    return cy_exact2(vec2<f32>(r * sc.y, r * sc.x));
}

struct CameraRay {
    origin: vec3<f32>,
    direction: vec3<f32>,
};

// The filter's table at step i of 1025 (FrameParams::filter_table).
fn filter_step(i: u32) -> f32 {
    return select(frame.filter_table[min(i, 1023u) >> 2u][i & 3u], frame.filter_last, i >= 1024u);
}

// FilterSampler::place's along for a Gaussian or a Blackman-Harris: a
// draw through the inverse of the filter's distribution over one half,
// read between its steps, on the side of the centre the draw falls.
fn filter_along(u: f32) -> f32 {
    let signed = 2.0 * u - 1.0;
    let at = abs(signed) * 1024.0;
    let i = min(u32(at), 1023u);
    let t = at - f32(i);
    let a = filter_step(i);
    let offset = a + cy_exact((filter_step(i + 1u) - a) * t);
    return 0.5 + bitcast<f32>((bitcast<u32>(offset) & 0x7FFFFFFFu) | (bitcast<u32>(signed) & 0x80000000u));
}

// FilterSampler::place: where in or around its pixel a sample goes, for its
// jitter `u` - where the jitter falls for one pixel's box; spread about the
// centre by a box's width (CAMERA_FILTER_BOX); each axis through the
// filter's table for a Gaussian or a Blackman-Harris (CAMERA_FILTER_TABLE).
fn filter_place(u: vec2<f32>) -> vec2<f32> {
    let flags = frame.camera_flags;
    if (FEATURE_FILTER && (flags & CAMERA_FILTER_TABLE) != 0u) {
        return vec2<f32>(filter_along(u.x), filter_along(u.y));
    }
    if (FEATURE_FILTER && (flags & CAMERA_FILTER_BOX) != 0u) {
        return 0.5 + cy_exact2((u - 0.5) * frame.filter_width);
    }
    return u;
}

// Camera::generate_ray (camera.rs) for a point on the film, (0, 0) the
// picture's top left and (1, 1) its bottom right: through the pinhole, its
// film shifted (CAMERA_SHIFT) or not; or, through a thin lens
// (CAMERA_THIN_LENS), from a point on its opening drawn on the lens
// dimensions (sampler::dim::LENS, after the pixel's jitter) towards where
// the pinhole's ray meets the focus plane; or parallel, from the film
// plane (CAMERA_ORTHOGRAPHIC); or every way round, the film's width the
// horizon and its height from the zenith down (CAMERA_EQUIRECTANGULAR). The time
// the processor draws with the shutter open (sampler::dim::TIME) is a
// Sobol' dimension, no state, and nothing reads a ray's time; no camera
// opens its shutter.
fn camera_ray(film: vec2<f32>, s: ptr<function, Sampler>) -> CameraRay {
    let sx = 2.0 * film.x - 1.0;
    let sy = 1.0 - 2.0 * film.y;
    let flags = frame.camera_flags;
    let lens = FEATURE_THIN_LENS && (flags & CAMERA_THIN_LENS) != 0u;
    let shifted = FEATURE_SHIFT && (flags & CAMERA_SHIFT) != 0u;
    let parallel = FEATURE_ORTHOGRAPHIC && (flags & CAMERA_ORTHOGRAPHIC) != 0u;
    let all_round = FEATURE_EQUIRECTANGULAR && (flags & CAMERA_EQUIRECTANGULAR) != 0u;
    if (lens || shifted || parallel || all_round) {
        var start = vec3<f32>(0.0);
        var towards: vec3<f32>;
        if (parallel) {
            start = vec3<f32>(cy_exact(sx * frame.half_width), cy_exact(sy * frame.half_height), 0.0);
            towards = vec3<f32>(0.0, 0.0, -1.0);
        } else if (all_round) {
            let phi = cy_sincos(cy_exact(sx * CY_PI));
            let theta = cy_sincos(cy_exact(film.y * CY_PI));
            towards = vec3<f32>(cy_exact(theta.x * phi.x), theta.y, -cy_exact(theta.x * phi.y));
        } else {
            // Without CAMERA_SHIFT the shift is nought, and sx + 0 is sx.
            let d = vec3<f32>(cy_exact((sx + 2.0 * frame.shift_x) * frame.half_width), cy_exact((sy + 2.0 * frame.shift_y) * frame.half_height), -1.0);
            if (lens) {
                start = vec3<f32>(cy_exact2(aperture_point(sampler_lens(s)) * frame.lens_radius), 0.0);
                let t_focus = cy_div(frame.focus_distance, -d.z);
                towards = normalized(cy_exact3(d * t_focus) - start);
            } else {
                towards = normalized(d);
            }
        }
        return CameraRay(rows_point(frame.camera_to_world, start), normalized(rows_vector(frame.camera_to_world, towards)));
    }
    let camera_dir = normalized(vec3<f32>(sx * frame.half_width, sy * frame.half_height, -1.0));
    let origin = rows_point(frame.camera_to_world, vec3<f32>(0.0));
    let direction = normalized(rows_vector(frame.camera_to_world, camera_dir));
    return CameraRay(origin, direction);
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
    let j = filter_place(sampler_pixel_jitter(&s));
    let film = vec2<f32>((f32(x) + j.x) / f32(frame.width), (f32(y) + j.y) / f32(frame.height));
    camera_film = film;
    // cy_exact's zero from a word the compiler cannot see is one (the
    // frame's mode never sets its top bit): a program sets it too, but a
    // variant without programs would otherwise know it, and the camera's
    // arithmetic would fuse there as it does not in the whole kernel.
    cy_zero = frame.mode & 0x80000000u;
    let ray = camera_ray(film, &s);
    // A spectral render's four wavelengths, one set for every path this
    // sample traces: the camera's, the split's reflection, the floor's
    // (each a trace of its own on the processor, drawing the same
    // wavelength dimension).
    spectral_on = FEATURE_SPECTRAL && (frame.mode & MODE_SPECTRAL) != 0u;
    if (spectral_on) {
        spectral_base = frame.spectral_base;
        spectral_begin(sampler_wavelength(&s));
    }

    var st = path_start(ray.origin, ray.direction, frame.max_bounces, false, false);
    var background = vec3<f32>(0.0);
    var coverage = 0.0;
    // The floor a path ended on, and whether there is one.
    var floor: FloorSeen;
    var on_floor = false;
    // Whether `st` is the floor's reflection's path; whether that escaped
    // at its first segment, and how far that went.
    var mirror = false;
    // Whether `st` is the path towards the product from a floor that shows
    // the product's light (#171, light_from_the_product), the light the
    // floor would get in the open there, and the floor's hit and the path
    // that met it as the floor's reflection takes them up once it is done
    // (`floor_resume`: the next segment is the floor's again, from its
    // reflection).
    var product = false;
    var product_open = 0.0;
    var floor_resume = false;
    var floor_raw: Surface;
    var floor_slot = 0u;
    var floor_max_bounces = 0u;
    var floor_reflected = false;
    var floor_alone_at = false;
    var floor_wavelength_at = 0.0;
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
    // The camera's coverage, settled at the first cutout it sees as the
    // light through every surface along the ray (camera_coverage), below
    // nought until then; and whether a shadow catcher lies behind those
    // cutouts, whose shadow's coverage is added where the path reaches it.
    var settled = -1.0;
    var catcher_behind = false;
    // The camera's wavelength, where its path ended on a floor whose
    // reflection then took its place.
    var floor_wavelength = 0.0;
    var floor_alone = false;
    // Where the segment set out, kept where a program drives a cutout: its
    // light sample's shadow rays meeting one whose opacity the pass does not
    // know, the segment is run again from there, what its walks have learnt
    // of the opacities kept (`keep_cut`), so the vertex is no turn of the
    // segment's loop and the driver keeps nothing over it.
    var start_st: PathState;
    var start_s: Sampler;
    var keep_cut = false;
    var out: PathOut;
    loop {
        // One segment of the path: `ended` false where the path goes on.
        // Its loop takes it a step at a time (`phase`, PH_*), and in each
        // turn, past the program and the segment's setting out, makes the
        // walk along a ray a step asked for, if one did (walk_ray, its one
        // place here): a later step stops where it asks and the next turn
        // takes up there, with what the walk found; the segment's own ray is
        // walked in the turn that sets it out, as a turn the driver keeps
        // everything over costs the whole kernel a fifth of its speed. A pass
        // of the segment may also be run more than once. Where a walk - a
        // shadow ray, the camera's coverage, the floor's shadow - meets a
        // cutout a program drives whose opacity this pass has not worked out
        // yet, the pass is thrown away: the path and the sampler are put back
        // as they were where the segment's ray was traced, or where it stood
        // at its hit, or - past the light sample's shadow rays - where the
        // segment set out; the program runs, at the one place a program runs;
        // and the pass runs again, the opacity known (crossing_opacity). Each
        // pass meets the same crossings in the same order, so the opacities
        // are remembered by their place in it.
        var ended = true;
        var phase = PH_START;
        // A function waiting on a walk: the turn of its loop it waits at, the
        // step within it, and a colour it carries over.
        var at_i = 0u;
        var at_step = 0u;
        var at_carried = vec3<f32>(0.0);
        // The walk asked for, made at the walk's place in this turn or the
        // next.
        var w: Walk;
        var walking = false;
        var want = WANT_NONE;
        if (keep_cut) {
            want = WANT_CROSSING;
        } else {
            cut_known = 0u;
        }
        keep_cut = false;
        cut_wanted = false;
        if (FEATURE_CUT_PROGRAMS && (frame.glass_flags & SCENE_CUT_PROGRAMS) != 0u) {
            start_st = st;
            start_s = s;
        }
        var seg_hit: RayIntersection;
        // A walk's flight drawn before the segment's ray was traced: whether
        // it was, each channel's chance and the optical depth (ic:975);
        // whether the ray was traced only as far as it goes; and, where it
        // was traced in full, the sampler before the draws, put back if the
        // ray then leaves the part, where the processor draws none.
        var seg_drawn = false;
        var seg_short = false;
        var seg_chance = vec3<f32>(0.0);
        var seg_depth = 0.0;
        var seg_pre_s: Sampler;
        // The light sample of the vertex the segment ends at, or of a
        // collision in a medium (`colliding`): direct_light's one call.
        var light: LightSampleAt;
        var colliding = false;
        // The floor's mirror ray while its nearness is walked: its way, how
        // squarely it leaves the floor and the nearness.
        var mirror_dir = vec3<f32>(0.0);
        var mirror_rise = 0.0;
        var mirror_near = 1.0;
        // bevel_normal's place while its probes are walked.
        var bevel_at: BevelAt;
        var seg_st: PathState;
        var seg_s: Sampler;
        var hit_st: PathState;
        var hit_s: Sampler;
        var hit_next = 0u;
        var seg_raw: Surface;
        var seg_surf: Surface;
        // The shading normal at the segment's hit with its edges rounded
        // (bevel_normal), what its bump bends; and whether the normal the
        // material leaves is to be turned again so its mirror reflection
        // stays above the surface, once, as the hit is taken up (#170).
        var seg_rounded: vec3<f32>;
        var seg_turn = false;
        var material: ProgShaded;
        let programs_cut = FEATURE_CUT_PROGRAMS && (frame.glass_flags & SCENE_CUT_PROGRAMS) != 0u;
        if (FEATURE_PRODUCT_LIGHT && floor_resume) {
            seg_raw = floor_raw;
            seg_hit.instance_custom_data = floor_slot;
            phase = PH_FLOOR;
            at_step = 8u;
        }
        loop {
            var found: WalkEnd;
            // ---- The one place a material's program runs (docs/07 §7) ----
            // The material at the segment's hit (ic:1626-1710), or a cutout's
            // opacity where a walk crossed it, as a shadow ray sees it
            // (shading_point_along with the default spread, ic:3404).
            if (FEATURE_PROGRAMS && want != WANT_NONE) {
                var at = seg_hit;
                var surf = seg_surf;
                var along = st.direction;
                var ray = st.ray;
                var spread = frame.pixel_spread;
                var rounded = seg_rounded;
                if (want == WANT_CROSSING) {
                    at = cut_hit;
                    along = cut_direction;
                    surf = crossing_surface(cut_hit, cut_direction);
                    ray = RAY_SHADOW;
                    spread = DEFAULT_PIXEL_SPREAD;
                    rounded = surf.shading;
                }
                let shaded = material_at(at, surf, along, ray, spread, rounded);
                if (want == WANT_CROSSING) {
                    cut_known_opacity[cut_known] = shaded.material.geometry_opacity;
                    cut_known += 1u;
                } else {
                    material = shaded;
                }
                want = WANT_NONE;
            }
            // ---- The segment sets out ----
            if (phase == PH_START) {
                // Asked once where a path sets out: by a walk where anything
                // transmits, none where nothing does; along the line behind
                // it.
                if (st.fresh) {
                    st.fresh = false;
                    if (FEATURE_GLASS && (frame.glass_flags & SCENE_TRANSMITS) != 0u) {
                        w = walk_new(WALK_MEDIA, st.origin, -st.direction, 3.0e38);
                        walking = true;
                        phase = PH_MEDIA;
                    } else {
                        var none: Media;
                        st.media = none;
                    }
                }
            }
            if (phase == PH_START) {
                // A walk through a medium that scatters draws its next
                // flight before its ray is traced, and traces it only as
                // far as the flight goes (ic:968-1001): most flights end
                // inside the part. Not past a seeing bounce, whose lamps
                // are looked for along the whole ray: its flight is the
                // one the processor draws after the ray (ic:1110),
                // drawn here for one call site, the sampler put back
                // where the ray leaves the part.
                seg_drawn = false;
                seg_short = false;
                var reach = 3.0e38;
                if (FEATURE_SCATTERING || FEATURE_SUBSURFACE) {
                    let top = media_top(st.media, NO_MEDIUM);
                    if (top != NO_MEDIUM && medium_scatters(st.media.media[top])) {
                        seg_short = !st.seen.valid;
                        seg_pre_s = s;
                        if (seg_short) {
                            st.walk_steps += 1u;
                            if (st.walk_steps > MAX_WALK_STEPS) {
                                break;
                            }
                        }
                        let walk = medium_scattering(st.media.media[top]);
                        // Guided, on the walk the path came in on: its
                        // flight stretched as its last turn says, and its
                        // first flight looking further, for the surface
                        // opposite the one it came in by.
                        let guiding = FEATURE_SUBSURFACE && st.guide.solid != NO_WALK && st.guide.solid == st.media.media[top].solid;
                        var stretch = 1.0;
                        if (guiding) {
                            stretch = md_guide_stretch(st.guide);
                        }
                        let pick = sampler_next_unstructured(&s);
                        let along = sampler_next_unstructured(&s);
                        let flight = md_walk_flight(st.throughput, walk.albedo, walk.sigma, stretch, pick, along);
                        seg_chance = flight.chance;
                        seg_depth = flight.depth;
                        if (seg_short) {
                            reach = flight.depth;
                            if (guiding && !st.guide.flown) {
                                reach = max(flight.depth, md_guide_look(walk.sigma));
                            }
                        }
                        seg_drawn = true;
                    }
                }
                // The floor's mirror ray's first hit, found before its path was
                // set up; else the segment's ray, walked.
                if (have_next && !seg_short) {
                    found.hit = next;
                    have_next = false;
                } else {
                    have_next = false;
                    w = walk_new(WALK_FIRST, st.origin, st.direction, reach);
                    walking = true;
                }
                phase = PH_RAY;
            }
            // ---- The one place a walk along a ray is made (docs/07 D5) ----
            if (walking) {
                found = walk_ray(w);
                walking = false;
                // Past a cutout a program drives whose opacity this pass does
                // not know: the pass thrown away, back to where the segment's
                // ray was traced or to its hit, the program run below.
                if (cut_wanted) {
                    cut_wanted = false;
                    want = WANT_CROSSING;
                    at_i = 0u;
                    at_step = 0u;
                    if (phase >= PH_HIT) {
                        st = hit_st;
                        s = hit_s;
                        cut_next = hit_next;
                        phase = PH_HIT;
                    } else if (FEATURE_PRODUCT_LIGHT && floor_resume) {
                        st = start_st;
                        s = start_s;
                        cut_next = 0u;
                        at_step = 8u;
                    } else {
                        st = seg_st;
                        s = seg_s;
                        cut_next = 0u;
                        phase = PH_TRACED;
                    }
                    continue;
                }
            }
            // ---- The media the path starts inside (enclosing_media) ----
            if (FEATURE_GLASS && phase == PH_MEDIA) {
                st.media = found.media;
                phase = PH_START;
                continue;
            }
            if (phase == PH_RAY) {
                seg_hit = found.hit;
                if (programs_cut) {
                    seg_st = st;
                    seg_s = s;
                }
                cut_next = 0u;
                at_i = 0u;
                at_step = 0u;
                phase = PH_TRACED;
            }
            // ---- The segment's ray traced ----
            if (phase == PH_TRACED) {
                let hit = seg_hit;
                let open = hit.kind == RAY_QUERY_INTERSECTION_NONE;
                // The lamps met on the way, and the sky where the ray escapes;
                // but a flight drawn and traced short that meets nothing
                // collides where it ends, and meets no sky.
                var sky = vec3<f32>(0.0);
                if (!(seg_short && open)) {
                    let met = meet_and_escape(&st, hit, at_i, at_step == 1u, found);
                    if (met.wants) {
                        w = met.walk;
                        at_i = met.i;
                        at_step = 1u;
                        walking = true;
                        continue;
                    }
                    sky = met.carried;
                }
                // The surface met, as Embree's resolve builds it: here, for
                // the walk's look at the surface opposite as well as for the
                // vertex below (one call, for the kernel's size).
                var raw: Surface;
                if (!open) {
                    raw = surface_of(hit);
                }
                // ---- The random walk (ic:1066-1256) ----
                // Inside a medium that scatters, the path may collide before
                // the surface ahead: a flight drawn in one channel, chosen by
                // the path's throughput times the medium's albedo, weighed
                // over all three; one that reaches the surface weighed by what
                // survives it. At a collision, Russian roulette once little is
                // carried, the lights sampled directly where the medium's
                // surface bends nothing and it is the only one the path is
                // in, and a turn by the phase function: the next segment.
                if (FEATURE_SCATTERING || FEATURE_SUBSURFACE) {
                    let top = media_top(st.media, NO_MEDIUM);
                    if (seg_drawn && open && !seg_short) {
                        s = seg_pre_s;
                    }
                    if (top != NO_MEDIUM && medium_scatters(st.media.media[top]) && (!open || seg_short)) {
                        let medium = st.media.media[top];
                        if (!seg_short) {
                            st.walk_steps += 1u;
                            if (st.walk_steps > MAX_WALK_STEPS) {
                                break;
                            }
                        }
                        let walk = medium_scattering(medium);
                        // Guided, on the walk the path came in on (ic:1103-1123):
                        // before its first turn, the surface opposite, if the
                        // first flight saw it within ten mean free paths.
                        let guiding = FEATURE_SUBSURFACE && st.guide.solid != NO_WALK && st.guide.solid == medium.solid;
                        var turned = false;
                        if (guiding) {
                            if (!st.guide.flown) {
                                var opposite = -1.0;
                                if (!open && scene_instances[hit.instance_custom_data].instance == st.guide.solid && hit.t <= md_guide_look(walk.sigma)) {
                                    let d = md_dot(raw.position - st.guide.entry, -st.guide.normal);
                                    if (d > 0.0) {
                                        opposite = d;
                                    }
                                }
                                st.guide.opposite = opposite;
                            }
                            turned = st.guide.turned;
                        }
                        let chance = seg_chance;
                        let depth = seg_depth;
                        // Where the surface ahead is: where the ray met it, or
                        // past a flight traced short, further than it goes.
                        let surface_at = select(hit.t, bitcast<f32>(0x7F800000u), open);
                        var lost = false;
                        let collided = depth < surface_at;
                        var t = st.throughput;
                        // What survives to the collision or the surface,
                        // and the density of ending there: one call for
                        // both.
                        let end = md_walk_end(st.guide, turned, chance, walk.sigma, select(surface_at, depth, collided), collided);
                        let kept = end.xyz;
                        let density = end.w;
                        if (density <= 0.0) {
                            lost = true;
                        } else if (collided) {
                            t = t * (cy_exact3(cy_exact3(walk.albedo * walk.sigma) * kept) * cy_div(1.0, density));
                            let carried = max(max(t.x, t.y), t.z);
                            if (carried < 0.1) {
                                if (sampler_next_unstructured(&s) >= carried) {
                                    lost = true;
                                } else {
                                    t = t * cy_div(1.0, carried);
                                    st.roulette = cy_div(st.roulette, carried);
                                }
                            }
                        } else {
                            t = t * (kept * cy_div(1.0, density));
                        }
                        st.throughput = t;
                        if (lost || !(is_finite(t.x) && is_finite(t.y) && is_finite(t.z)) || all(t == vec3<f32>(0.0))) {
                            break;
                        }
                        if (collided) {
                            let at = st.origin + cy_exact3(st.direction * depth);
                            st.lit_from = NO_SOLID;
                            if (FEATURE_SCATTERING && !medium.doorway && abs(medium.ior - 1.0) < 1e-3 && st.media.len == 1u && (frame.lamp_flags & LAMPS_UNSAMPLED) == 0u) {
                                // Its light sample (medium_direct_light), by
                                // direct_light's one call below: its draws
                                // here, before the turn's, as the processor
                                // draws them - the sky's, the lamp's choice
                                // by power and the lamp's where it lights
                                // the medium's part.
                                light.wanted = true;
                                light.u_sky = sampler_light_sample(&s);
                                light.choice.ok = false;
                                if (FEATURE_LAMPS && frame.light_count > 0u) {
                                    let choice = lamps_select(frame.light_count, sampler_light_select(&s));
                                    light.choice = choice;
                                    if (choice.ok && lamp_lights(scene_lights[choice.index], medium.solid)) {
                                        light.u_lamp = sampler_light_sample(&s);
                                    }
                                }
                                light.medium = true;
                                light.surf.position = at;
                                light.wo = st.direction;
                                light.instance = medium.solid;
                                light.facing = vec3<f32>(0.0);
                                light.throughput = st.throughput;
                                light.ceiling = light_ceiling(st.specular_chain, st.bounce, st.roulette);
                                light.sigma = walk.sigma;
                                light.g = walk.g;
                                st.lit_from = medium.solid;
                                st.sky_facing = vec3<f32>(0.0);
                            }
                            // The turn: guided about the entry's normal, or
                            // by the phase function about the way the path
                            // was going; one call of the turn.
                            // Through a local: naga's SPIR-V writer cannot
                            // pass a pointer to a member.
                            var guide = st.guide;
                            var pick: vec3<f32>;
                            if (guiding) {
                                pick = md_guide_pick(&guide, at, &s);
                            } else {
                                let u0 = sampler_next_unstructured(&s);
                                let u1 = sampler_next_unstructured(&s);
                                pick = vec3<f32>(u0, u1, 0.0);
                            }
                            let about_normal = guiding && guide.guided;
                            let axis = select(st.direction, guide.normal, about_normal);
                            let cos_theta = select(md_phase_cosine(walk.g, pick.x), pick.z, about_normal);
                            let onward = md_turned(axis, cos_theta, pick.y);
                            // The phase function's density for the turn: the
                            // sky's weight past the medium, and the guide's.
                            st.lit_pdf = md_phase(md_dot(st.direction, onward), walk.g);
                            if (guiding) {
                                md_guide_keep(&guide, onward, st.lit_pdf, pick.z);
                                st.guide = guide;
                            }
                            st.least_pdf = min(st.least_pdf, st.lit_pdf);
                            st.origin = at;
                            st.direction = onward;
                            st.prev_pdf = PDF_INFINITE;
                            st.specular_chain = false;
                            st.through_glass = GLASS_SOLID;
                            st.seen.valid = false;
                            ended = false;
                            if (light.wanted) {
                                colliding = true;
                                cut_next = hit_next;
                                at_i = 0u;
                                at_step = 0u;
                                phase = PH_HIT;
                                continue;
                            }
                            break;
                        }
                    }
                }
                if (hit.kind == RAY_QUERY_INTERSECTION_NONE) {
                    if (st.bounce == 0u) {
                        if (mirror) {
                            escaped = true;
                        } else if (!st.reflected && !product) {
                            background = sky;
                            // Past cutouts, the coverage they settled; else
                            // none (ic:1308).
                            coverage = max(settled, 0.0);
                        }
                    }
                    break;
                }
                // A part this kind of ray does not see (ic:1344-1369) is
                // passed as through a hole, nothing else changed.
                let seen_flags = scene_instances[hit.instance_custom_data].flags;
                if (FEATURE_UNSEEN && (seen_flags & (INSTANCE_NO_CAMERA | INSTANCE_NO_DIFFUSE | INSTANCE_NO_GLOSSY | INSTANCE_NO_TRANSMISSION)) != 0u
                    && st.crossings < MAX_CUTOUT_CROSSINGS && unseen_by(st.ray, st.bounce, st.in_floor, seen_flags)) {
                    st.crossings += 1u;
                    let place = hit_place(hit);
                    let g = select(place.geometric, -place.geometric, dot(st.direction, place.geometric) > 0.0);
                    st.origin = offset_at(place.position, -g, place.magnitude);
                    ended = false;
                    break;
                }
                if (st.bounce == 0u) {
                    if (mirror) {
                        depth = hit.t;
                    } else if (!st.reflected && !product) {
                        coverage = select(1.0, settled, settled >= 0.0);
                    }
                    // Held out (ic:1377-1382): a hole where a path first meets
                    // it, nothing past it, and where it is the camera's, no
                    // alpha.
                    if (FEATURE_UNSEEN && (seen_flags & INSTANCE_HOLDOUT) != 0u) {
                        if (!mirror && !st.reflected && !product) {
                            coverage = 0.0;
                        }
                        break;
                    }
                }
                let inst = scene_instances[hit.instance_custom_data];
                // Anything but the lit medium's own surface is a new event,
                // which its collision's light sample did not see past; a
                // lamp's stand-in passed below it did (ic:1338).
                if (FEATURE_SCATTERING && st.lit_from != NO_SOLID && st.lit_from != inst.solid) {
                    let stand_in_shows = st.specular_chain && ((frame.lamp_flags & LAMPS_IN_MIRRORS) == 0u || (st.bounce == 0u && !st.reflected));
                    if (stand_in_shows || (inst.flags & INSTANCE_STAND_IN) == 0u) {
                        st.lit_from = NO_SOLID;
                    }
                }
                seg_raw = raw;
                at_step = 0u;
                phase = select(PH_BODY, PH_FLOOR, FEATURE_FLOOR && inst.catcher != NO_CATCHER);
            }
            // ---- A floor met ----
            if (FEATURE_FLOOR && phase == PH_FLOOR) {
                let raw = seg_raw;
                let inst = scene_instances[seg_hit.instance_custom_data];
                let c = scene_catchers[inst.catcher];
                if (st.catcher_view) {
                    // Seen as the camera would see it (ic:1396-1471): what it
                    // stands in front of, by the rule an escaping path follows,
                    // with its shadow and its reflection on it; and nothing
                    // further along this path. Never in the reflection's path,
                    // whose catcher_view is false.
                    let view = st.direction;
                    let normal = select(-raw.geometric, raw.geometric, dot(raw.geometric, -view) > 0.0);
                    // catch_shadow up to the reflection (ic:2888-2925): what is
                    // behind the floor darkened towards the shadow's colour by
                    // the share of the light the scene blocks there - a cast
                    // shadow, or ambient occlusion, each a walk - and that
                    // share.
                    var share = 0.0;
                    if (at_step == 0u) {
                        floor.behind = past_the_scene(st.specular_chain, st.bounce == 0u && !st.reflected, st.in_floor, st.origin, view);
                        at_step = 2u;
                        if (c.shadows != 0u) {
                            // The shadow's draws, and the open light the
                            // product's light is measured against (#171).
                            let shadow = shadow_at(raw.position, raw.magnitude, normal, st.bounce, &s, inst.instance, c.occlusion_only == 0u, c.product_light != 0u);
                            product_open = shadow.open;
                            if (c.occlusion_only != 0u) {
                                w = occlusion_walk(raw.position, raw.magnitude, normal, c.occlusion_radius, &s);
                                walking = true;
                                at_step = 1u;
                                continue;
                            }
                            if (shadow.walks) {
                                w = shadow.walk;
                                walking = true;
                                at_step = 1u;
                                continue;
                            }
                            share = shadow.share;
                            sampler_begin_bounce(&s, st.bounce);
                        }
                    } else if (at_step == 1u) {
                        if (c.occlusion_only != 0u) {
                            share = blocked_of(found);
                        } else {
                            share = shadow_at_of(found);
                            sampler_begin_bounce(&s, st.bounce);
                        }
                        at_step = 2u;
                    }
                    if (at_step == 2u) {
                        share = clamp(share, 0.0, 1.0);
                        let caught = Caught(lerp3(floor.behind, c.shadow_color.xyz, share), share);
                        floor = FloorSeen(st.radiance, st.throughput, st.rad4, st.tp4, st.alone, st.bounce, floor.behind, caught, c, normal, view, 0.0, 1.0, !st.reflected);
                        on_floor = true;
                        at_step = 8u;
                        // What the product sends onto the floor (#171,
                        // ic:3019-3024): where the floor shows it and its
                        // shadows are on.
                        if (FEATURE_PRODUCT_LIGHT && c.product_light != 0u && c.shadows != 0u) {
                            at_step = 6u;
                        }
                    }
                    // light_from_the_product (ic:3065-3103): against the light
                    // the floor would get in the open, one direction from the
                    // floor by its cosine (two draws of the open stream);
                    // where it meets the product - not the floor, not clear
                    // glass, whose light straight on is the shadow's - the
                    // light coming back along it is a path of its own, four
                    // bounces at most, seeing no floor as the camera would.
                    // Its first hit is walked here and handed to it.
                    if (FEATURE_PRODUCT_LIGHT && at_step == 6u) {
                        at_step = 8u;
                        if (product_open > 0.0) {
                            let u0 = sampler_next_unstructured(&s);
                            let u1 = sampler_next_unstructured(&s);
                            mirror_dir = to_world(frame_of(normal), sample_cosine_hemisphere(vec2<f32>(u0, u1)));
                            w = walk_new(WALK_FIRST, offset_at(raw.position, normal, raw.magnitude), mirror_dir, 3.0e38);
                            walking = true;
                            at_step = 7u;
                            continue;
                        }
                    }
                    if (FEATURE_PRODUCT_LIGHT && at_step == 7u) {
                        at_step = 8u;
                        let first = found.hit;
                        if (first.kind != RAY_QUERY_INTERSECTION_NONE) {
                            let met = scene_instances[first.instance_custom_data];
                            if (met.catcher == NO_CATCHER && (scene_materials[met.material].glass & GLASS_PANE) == 0u) {
                                floor_raw = raw;
                                floor_slot = seg_hit.instance_custom_data;
                                floor_max_bounces = st.max_bounces;
                                floor_reflected = st.reflected;
                                floor_alone_at = st.alone;
                                floor_wavelength_at = st.wavelength;
                                st = path_start(offset_at(raw.position, normal, raw.magnitude), mirror_dir, min(st.max_bounces, 4u), st.reflected, true);
                                product = true;
                                next = first;
                                have_next = true;
                                ended = false;
                                break;
                            }
                        }
                    }
                    if (at_step == 8u) {
                        if (c.reflections <= 0.0) {
                            break;
                        }
                        // floor_reflection up to its path (ic:2982-3026).
                        let ray = mirror_ray(c, normal, view, &s);
                        if (!ray.ok) {
                            break;
                        }
                        mirror_dir = ray.direction;
                        mirror_rise = ray.rise;
                        mirror_near = 1.0;
                        at_step = 4u;
                        if (c.fade == FADE_BY_DISTANCE) {
                            // Where some part casts no shadow, measured over
                            // every part standing on the floor (nearness_at);
                            // else as a shadow.
                            if (FEATURE_PASSES_PARTS && (frame.lamp_flags & LAMPS_SHADOWLESS) != 0u) {
                                w = nearness_walk(raw.position, raw.magnitude, normal, c.reflection_fade, &s);
                            } else {
                                w = occlusion_walk(raw.position, raw.magnitude, normal, c.reflection_fade, &s);
                            }
                            walking = true;
                            at_step = 3u;
                            continue;
                        }
                    } else if (at_step == 3u) {
                        if (FEATURE_PASSES_PARTS && (frame.lamp_flags & LAMPS_SHADOWLESS) != 0u) {
                            mirror_near = found.through.x;
                        } else {
                            mirror_near = blocked_of(found);
                        }
                        if (mirror_near <= 0.0) {
                            break;
                        }
                        at_step = 4u;
                    }
                    // A short reflection is gone above its fade, and one that
                    // never fades meets nothing over most of the floor: one
                    // intersection says so first, walked.
                    let origin = offset_at(raw.position, normal, raw.magnitude);
                    if (at_step == 4u) {
                        w = walk_new(WALK_FIRST, origin, mirror_dir, 3.0e38);
                        walking = true;
                        at_step = 5u;
                        continue;
                    }
                    if (at_step == 5u) {
                        let first = found.hit;
                        let met = first.kind != RAY_QUERY_INTERSECTION_NONE;
                        let reaches = c.fade == FADE_BY_DISTANCE || (met && !(c.fade == FADE_BY_HEIGHT && first.t * mirror_rise >= c.reflection_fade));
                        if (reaches) {
                            floor.rise = mirror_rise;
                            floor.near = mirror_near;
                            floor_wavelength = st.wavelength;
                            floor_alone = st.alone;
                            st = path_start(origin, mirror_dir, min(st.max_bounces, 3u), st.reflected, true);
                            mirror = true;
                            next = first;
                            have_next = true;
                            ended = false;
                        }
                    }
                    break;
                }
                // Met other than as the camera sees it: a floor that glows
                // darker toward the contact over its falloff, walked.
                var open = 1.0;
                if (c.bounce.w != 0.0 && c.bounce_falloff > 0.0) {
                    if (at_step == 0u) {
                        let towards = select(-raw.geometric, raw.geometric, dot(raw.geometric, -st.direction) > 0.0);
                        w = occlusion_walk(raw.position, raw.magnitude, towards, c.bounce_falloff, &s);
                        walking = true;
                        at_step = 1u;
                        continue;
                    }
                    open = 1.0 - blocked_of(found);
                }
                let met = floor_met(&st, c, raw, open);
                if (met == MET_END) {
                    break;
                }
                if (met == MET_PASSED) {
                    ended = false;
                    break;
                }
                phase = PH_BODY;
            }
            // ---- On to the surface's material ----
            if (phase == PH_BODY) {
                let hit = seg_hit;
                let raw = seg_raw;
                let inst = scene_instances[hit.instance_custom_data];
                // Beer-Lambert across the segment just travelled, in the medium
                // present (ic:1527-1556): only between two surfaces.
                let top = media_top(st.media, NO_MEDIUM);
                if (FEATURE_GLASS && top != NO_MEDIUM && !(FEATURE_SUBSURFACE && st.media.media[top].doorway)) {
                    let medium = scene_materials[st.media.media[top].material];
                    if ((medium.glass & GLASS_VOLUME) != 0u) {
                        // What the medium gives off along the way, a lit
                        // light guide's glow, before what it absorbs of the
                        // rest; times the square of its index, as light born
                        // inside a denser medium leaves it (ic:1527-1545).
                        let ior = st.media.media[top].ior;
                        let glow = emitted_over(medium, hit.t) * max(ior * ior, 1.0);
                        if (any(glow != vec3<f32>(0.0)) && !(FEATURE_SPECTRAL && spectral_on)) {
                            let found_at = select(st.bounce - 1u, 0u, st.bounce == 0u);
                            st.radiance += clamp_light(st.throughput * glow, light_ceiling(st.specular_chain, found_at, st.roulette));
                        }
                        if (FEATURE_SPECTRAL && spectral_on) {
                            // The glow, where there is one, then the spectrum
                            // of what RGB lets through over the stretch
                            // (ic:1545-1552), read from one place.
                            let found_at = select(st.bounce - 1u, 0u, st.bounce == 0u);
                            let transmitted = transmittance_over(medium, hit.t);
                            for (var k = select(1u, 0u, any(glow != vec3<f32>(0.0))); k < (2u | cy_zero); k++) {
                                let c4 = spectral_of(select(glow, transmitted, k == 1u), k == 0u);
                                if (k == 0u) {
                                    st.rad4 += cy_exact4(clamp_light4(st.tp4 * c4, light_ceiling(st.specular_chain, found_at, st.roulette)));
                                } else {
                                    st.tp4 = st.tp4 * c4;
                                }
                            }
                            let t4 = st.tp4;
                            if (!(is_finite(t4.x) && is_finite(t4.y) && is_finite(t4.z) && is_finite(t4.w)) || all(t4 == vec4<f32>(0.0))) {
                                break;
                            }
                        } else {
                            let t = st.throughput * transmittance_over(medium, hit.t);
                            st.throughput = t;
                            if (!(is_finite(t.x) && is_finite(t.y) && is_finite(t.z)) || all(t == vec3<f32>(0.0))) {
                                break;
                            }
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
                if (FEATURE_GLASS && (base.glass & GLASS_BOUNDARY) != 0u
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
                seg_surf = facing(raw, st.direction);
                seg_rounded = seg_surf.shading;
                var fresh: BevelAt;
                bevel_at = fresh;
                bevel_at.sum = seg_surf.shading * 1e-3;
                phase = PH_BEVEL;
            }
            // The material at this point (ic:1626-1710): its program run and
            // its bump applied, ahead of the cutout and the glow it may drive
            // and of the bounce limit, as the processor shades it there too;
            // past the limit only where the program drives what is read
            // before it.
            if (phase == PH_BEVEL) {
                let hit = seg_hit;
                let inst = scene_instances[hit.instance_custom_data];
                let base = scene_materials[inst.material];
                // Rounded edges on the surface a path meets first (ic:1680-1691):
                // their draws after the material's program (which draws none)
                // and before the cutout's, the bump bending what they leave;
                // each probe walked.
                if (FEATURE_BEVEL && (frame.glass_flags & SCENE_BEVEL) != 0u && st.bounce == 0u && base.edge_radius > 0.0) {
                    let rounded = bevel_normal_step(hit.instance_custom_data, seg_raw.position, seg_surf.shading, base.edge_radius, base.edge_samples, &s, &bevel_at, found);
                    if (rounded.wants) {
                        w = rounded.walk;
                        walking = true;
                        continue;
                    }
                    seg_rounded = rounded.normal;
                }
                material = ProgShaded(base, seg_rounded, vec3<f32>(0.0), 0.0, vec2<f32>(0.0));
                let program = scene_programs[inst.material];
                // A bumped surface's normal, and a rounded one, turned again
                // once bent (ic:1773-1784), as Cycles makes its reflection
                // valid on the normal the BSDF is given.
                seg_turn = (FEATURE_PROGRAMS && (program.flags & (PROGRAM_BUMPS | PROGRAM_NORMAL_UV | PROGRAM_NORMAL_PROJECTED)) != 0u) || (FEATURE_BEVEL && base.edge_radius > 0.0);
                if ((program.flags & PROGRAM_RUNS) != 0u && (st.bounce - st.glass_crossings < st.max_bounces || (program.flags & PROGRAM_AT_HIT) != 0u)) {
                    want = WANT_MAIN;
                }
                phase = PH_HIT;
                hit_next = cut_next;
                if (programs_cut) {
                    hit_st = st;
                    hit_s = s;
                }
                continue;
            }
            // ---- At the segment's hit, its material known ----
            // A cutout (ic:1710-1747): a hole, not a filter - the path meets
            // the surface or carries straight on, by a draw of the open
            // stream against its opacity, spending no bounce and changing no
            // weight. At the first one the camera sees, the pixel's coverage
            // is the light through every surface along the ray, averaged (a
            // walk, camera_coverage).
            let cutout = !colliding && FEATURE_CUTOUTS && (frame.glass_flags & SCENE_CUTOUTS) != 0u && material.material.geometry_opacity < 0.999;
            let covers = cutout && st.bounce == 0u && !st.reflected && !mirror && !product && settled < 0.0;
            if (phase == PH_HIT) {
                if (seg_turn) {
                    material.normal = valid_for_reflection(seg_surf.geometric, -st.direction, material.normal);
                    seg_turn = false;
                }
                if (covers) {
                    w = coverage_walk(offset_at(seg_raw.position, -seg_surf.geometric, seg_raw.magnitude), st.direction, material.material.geometry_opacity);
                    walking = true;
                    phase = PH_COVERED;
                    continue;
                }
                phase = PH_COVERED;
            }
            if (phase == PH_COVERED) {
                if (cutout) {
                    let opacity = material.material.geometry_opacity;
                    let beyond = offset_at(seg_raw.position, -seg_surf.geometric, seg_raw.magnitude);
                    if (covers) {
                        settled = found.through.x;
                        catcher_behind = found.stopped;
                        coverage = found.through.x;
                    }
                    if (st.crossings < MAX_CUTOUT_CROSSINGS && sampler_next_unstructured(&s) >= opacity) {
                        st.crossings += 1u;
                        st.origin = beyond;
                        ended = false;
                        break;
                    }
                }
                at_i = 0u;
                at_step = 0u;
                phase = PH_PAST;
            }
            // The light past a part its shadow rays pass, where the path
            // stops at it (ic:1753-1801).
            if (phase == PH_PAST) {
                if (FEATURE_PASSES_PARTS && (frame.lamp_flags & LAMPS_PASSES_PARTS) != 0u) {
                    let past = light_past_part(&st, seg_hit, seg_raw, material.material, at_i, at_step, at_carried, found);
                    if (past.wants) {
                        w = past.walk;
                        at_i = past.i;
                        at_step = past.step;
                        at_carried = past.carried;
                        walking = true;
                        continue;
                    }
                }
                at_step = 0u;
                phase = PH_GLOW;
            }
            // A glowing surface met from the front (ic:1813-1860), but by a
            // diffuse bounce where glows light nothing: weighed against the
            // light sample of the glowing parts the last vertex took, or past
            // glass a glossy bounce saw against the one its straight line
            // meets (a walk); a stand-in its face as its lamp lights, falling
            // off to its edges.
            if (phase == PH_GLOW) {
                let hit = seg_hit;
                let raw = seg_raw;
                let inst = scene_instances[hit.instance_custom_data];
                let lit = (frame.glass_flags & SCENE_LIGHT_MATERIALS) != 0u || st.ray != RAY_DIFFUSE;
                if (!colliding && FEATURE_GLOW && material_is_emissive(material.material) && dot(st.direction, raw.geometric) <= 0.0 && lit) {
                    // The light sample it is weighed against: the one taken where
                    // the path last scattered, of this point; or past glass a
                    // glossy bounce saw, the one taken there, of the glowing
                    // surface its straight line meets (one density for both, for
                    // the code's size).
                    var weigh = false;
                    var from_point = st.receiver_position;
                    var from_normal = st.receiver_normal;
                    var from_two_sided = st.receiver_two_sided;
                    var pdf = st.prev_pdf;
                    var met = GlowTarget(true, inst.emitter, hit.primitive_index, raw.position, raw.geometric);
                    var along = st.direction;
                    if (st.receiver && st.prev_pdf < PDF_INFINITE) {
                        weigh = true;
                    } else if (FEATURE_EMITTERS && st.seen.valid && st.seen.met && (frame.glass_flags & SCENE_EMITTERS) != 0u && st.seen.density < PDF_INFINITE) {
                        if (at_step == 0u) {
                            w = glow_walk(st.seen);
                            walking = true;
                            at_step = 1u;
                            continue;
                        }
                        met = glow_target_of(found);
                        weigh = met.ok;
                        from_point = st.seen.position;
                        from_normal = st.seen.normal;
                        from_two_sided = st.seen.two_sided;
                        pdf = st.seen.density;
                        along = st.seen.direction;
                    }
                    var weight = 1.0;
                    if (FEATURE_EMITTERS && weigh) {
                        let density = emitters_density(from_point, from_normal, from_two_sided, met.first, met.primitive);
                        if (density > 0.0) {
                            let to = met.position - from_point;
                            weight = mis_against_area(pdf, density, dot(to, to), dot(along, met.geometric));
                        }
                    }
                    var face = 1.0;
                    if (FEATURE_LAMPS && inst.light != NO_LIGHT) {
                        let lamp = scene_lights[inst.light];
                        if (lamp.kind == LIGHT_AREA) {
                            face = light_face_at(lamp, raw.position);
                        }
                    }
                    let found_at = select(st.bounce - 1u, 0u, st.bounce == 0u);
                    if (FEATURE_SPECTRAL && spectral_on) {
                        st.rad4 += cy_exact4(clamp_light4(st.tp4 * spectral_of(material_emission(material.material), true) * (weight * face), light_ceiling(st.specular_chain, found_at, st.roulette)));
                    } else {
                        st.radiance += clamp_light(st.throughput * material_emission(material.material) * (weight * face), light_ceiling(st.specular_chain, found_at, st.roulette));
                    }
                }
            }
            // The vertex, then its light sample (ic:2043-2224), or the
            // collision's (ic:4123): direct_light's one call, which walks its
            // shadow rays itself (walk_ray).
            var off: SplitOff;
            var shaded = SHADE_ON;
            if (!colliding) {
                light.wanted = false;
                shaded = shade_vertex(&st, &s, seg_hit, seg_surf, material, &off, &light);
            }
            if (light.wanted) {
                let direct = direct_light(&s, light);
                if (FEATURE_SCATTERING && light.medium) {
                    st.radiance += direct.total;
                } else if (FEATURE_SPECTRAL && spectral_on) {
                    // Through a dispersive surface the light came by the hero's
                    // index, and is the hero's alone (ic:2172-2193).
                    // The sum's colour, or each source's - the sky's, the lamp's,
                    // the glowing triangle's, in that order - from one place.
                    let apart = light.disperses && !light.alone;
                    let alone = spectral_hero_alone(light.tp4);
                    var contribution = vec4<f32>(0.0);
                    for (var k = 0u; k < (select(1u, 3u, apart) | cy_zero); k++) {
                        var lit = direct.total;
                        var by = light.tp4;
                        if (apart) {
                            lit = cy_exact3(direct.values[k] * direct.scales[k]);
                            by = select(light.tp4, alone, (direct.through & (1u << k)) != 0u);
                        }
                        contribution = contribution + cy_exact4(by * spectral_of(lit, true));
                    }
                    st.rad4 += cy_exact4(clamp_light4(contribution, light.ceiling));
                } else {
                    st.radiance += clamp_light(light.throughput * direct.total, light.ceiling);
                }
            }
            // Its shadow rays past a cutout a program drives whose opacity
            // this pass does not know: the segment run again (below).
            if (cut_wanted) {
                keep_cut = true;
                break;
            }
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
        if (keep_cut) {
            st = start_st;
            s = start_s;
            continue;
        }
        floor_resume = false;
        if (!ended) {
            continue;
        }
        // A path has ended. One that ended on a floor has the floor's light
        // added to what it had, unclamped, and at the camera's own hit the
        // backdrop it leaves showing and the coverage.
        var result = st.radiance;
        var wavelength = st.wavelength;
        var alone = st.alone;
        if (FEATURE_SPECTRAL && spectral_on) {
            result = spectral_rgb(st.rad4);
        }
        // The path towards the product, done (ic:3023): what it brought, as
        // irradiance over the open light (a cosine draw's is pi times the
        // radiance), times what is behind the floor, onto what the floor
        // shows; and the floor's segment taken up again at its reflection,
        // the path that met it as it was there.
        if (FEATURE_PRODUCT_LIGHT && product) {
            floor.caught.radiance += floor.behind * (finished(result) * cy_div(PI, product_open));
            product = false;
            floor_resume = true;
            st.direction = floor.view;
            st.max_bounces = floor_max_bounces;
            st.reflected = floor_reflected;
            st.in_floor = false;
            st.catcher_view = true;
            st.alone = floor_alone_at;
            st.wavelength = floor_wavelength_at;
            continue;
        }
        if (on_floor) {
            var caught = floor.caught;
            if (mirror) {
                caught = reflected_on(floor, finished(result), escaped, depth);
                wavelength = floor_wavelength;
                alone = floor_alone;
            }
            if (FEATURE_SPECTRAL && spectral_on) {
                // What the floor caught, then the backdrop it leaves
                // showing, read from one place.
                let backdrop = floor.bounce == 0u && floor.camera;
                for (var k = 0u; k < (select(1u, 2u, backdrop) | cy_zero); k++) {
                    let c4 = spectral_of(select(caught.radiance, floor.behind, k == 1u), true);
                    if (k == 0u) {
                        result = spectral_rgb(floor.rad4 + cy_exact4(floor.tp4 * c4));
                    } else {
                        background = spectral_rgb(floor.tp4 * c4 * (1.0 - caught.coverage));
                    }
                }
            } else {
                result = floor.radiance + floor.throughput * caught.radiance;
            }
            if (floor.bounce == 0u && floor.camera) {
                if (!(FEATURE_SPECTRAL && spectral_on)) {
                    background = floor.throughput * floor.behind * (1.0 - caught.coverage);
                }
                // Through cutouts, whose share is settled: the floor adds its
                // shadow's on the paths that reach it (ic:1452).
                coverage = select(caught.coverage, settled + caught.coverage, settled >= 0.0 && catcher_behind);
            }
            on_floor = false;
            mirror = false;
            escaped = false;
        }
        // The split's reflection: what it saw, times its share, joins the
        // camera's light; the camera's path goes on, or ends there.
        if (reflecting) {
            split_light += carried * finished(result) * split_share;
            st = parked;
            reflecting = false;
            if (parked_ends || (parked_roulette && !russian_roulette(&st, &s))) {
                var parked_result = st.radiance;
                if (FEATURE_SPECTRAL && spectral_on) {
                    parked_result = spectral_rgb(st.rad4);
                }
                out = PathOut(finished(parked_result + split_light), background, coverage, st.wavelength, spectral_hue(st.alone));
                break;
            }
            continue;
        }
        out = PathOut(finished(result + split_light), background, coverage, wavelength, spectral_hue(alone));
        break;
    }
    return out;
}

@compute @workgroup_size(8, 8)
fn transport_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let y = frame.row_base + id.y * frame.row_stride;
    if (id.x >= frame.width || y >= frame.height) {
        return;
    }
    let pixel = y * frame.width + id.x;
    let pixels = frame.width * frame.height;
    for (var i = 0u; i < frame.samples; i++) {
        let out = path(id.x, y, frame.sample_base + i);
        if ((frame.mode & 1u) != 0u) {
            // How luminous the film finds the light a dispersed path
            // carries (Pixel::accumulate_with's hue), 1 for any other.
            var hue = 1.0;
            if (FEATURE_SPECTRAL && spectral_on) {
                hue = max(out.hue, 1e-3);
            } else if (FEATURE_DISPERSION && out.wavelength > 0.0) {
                hue = max(luminance(dispersion_weight(out.wavelength)), 1e-3);
            }
            accumulate(pixel, out.radiance, out.background, out.coverage, hue, FEATURE_SPECTRAL && spectral_on);
        } else {
            let at = (i * pixels + pixel) * 2u;
            frame_samples[at] = vec4<f32>(out.radiance, out.coverage);
            // A spectral sample's hue, else its wavelength.
            frame_samples[at + 1u] = vec4<f32>(out.background, select(out.wavelength, out.hue, FEATURE_SPECTRAL && spectral_on));
        }
    }
}
