// The analytic lamps: tequila_render_core::light's area (rectangle and disc,
// with their face's falloff and fade and a grid's spread), point (a sphere
// with a radius), spot and distant (the sun, a delta or a disc) lights, one
// lamp at a time - its light sample, what a ray meets of it, the density a
// light sample gives a direction to it - and a node of the light hierarchy's
// importance from a point. lights.rs here writes the records; whatever the
// processor's lamp works out only from its own parameters (an area lamp's
// normal and area, its spherical rectangle's frame, a grid's tangent, a
// spot's cosines, a sun's disc) comes worked out, so it is the processor's
// number. No bindings: lamps.wgsl reads the scene's lamps.
//
// An IES lamp is not ported: its record is there for the selection, and its
// light sample is none (transport.rs's lamp_limits says so).
//#include onb

const LT_PI: f32 = 3.14159265358979323846;
const LT_TAU: f32 = 6.28318530717958647692;

// GpuLight's kind (lights.rs's KIND_*).
const LIGHT_AREA: u32 = 0u;
const LIGHT_POINT: u32 = 1u;
const LIGHT_SPOT: u32 = 2u;
const LIGHT_IES: u32 = 3u;
const LIGHT_DISTANT: u32 = 4u;

// GpuLight's flags (lights.rs's FLAG_*).
const LF_DISC: u32 = 1u;
const LF_TWO_SIDED: u32 = 2u;
const LF_ENABLED: u32 = 4u;
const LF_CAMERA: u32 = 8u;
const LF_DIFFUSE: u32 = 16u;
const LF_SPECULAR: u32 = 32u;
const LF_SHADOW: u32 = 64u;
const LF_CAN_BE_MET: u32 = 128u;
const LF_DELTA: u32 = 256u;
const LF_EVEN: u32 = 512u;
const LF_SPREAD_FULL: u32 = 1024u;
const LF_RECTANGLE: u32 = 2048u;
const LF_LINKS_ONLY: u32 = 4096u;
const LF_LINKS_ALL_BUT: u32 = 8192u;

// lights.rs's GpuLight, member for member (tests/layouts.rs holds the two
// to one layout).
struct GpuLight {
    // The lamp's place; w a point lamp's radius.
    position: vec4<f32>,
    // An area lamp's half extents; w each one's squared length, at least
    // 1e-12.
    half_u: vec4<f32>,
    half_v: vec4<f32>,
    // An area lamp's normal (AreaLight::normal), a spot's axis, the way a
    // sun's light travels; w an area lamp's area.
    normal: vec4<f32>,
    // Radiance in nits, intensity in candela, or illuminance in lux; w a
    // sun's 1 / (pi sin² of its half angle).
    radiance: vec4<f32>,
    // The spherical rectangle's frame (SphericalRectangle::build), for a
    // rectangle: its x and y, w the edge lengths, its z, and the corner it
    // starts from.
    rect_x: vec4<f32>,
    rect_y: vec4<f32>,
    rect_z: vec4<f32>,
    rect_origin: vec4<f32>,
    kind: u32,
    flags: u32,
    // spread_attenuation's tan(half the spread) and its normalization.
    tan_half: f32,
    spread_norm: f32,
    falloff: f32,
    fade: f32,
    // A spot's cos(outer) and cos(inner) and falloff exponent; a sun's
    // cos and sin of its half angle and its disc's density.
    cos_a: f32,
    cos_b: f32,
    exponent: f32,
    // LightSet's probabilities[i] and alias[i], and the leaf that holds it.
    probability: f32,
    alias_p: f32,
    alias_alt: u32,
    leaf: u32,
    // Its light links: where its instances start in the links buffer, and
    // how many (LF_LINKS_ONLY or LF_LINKS_ALL_BUT say which way).
    links: u32,
    link_count: u32,
    _pad: u32,
};

// lights.rs's GpuLightNode: LightNode as LightSet::hierarchy shows it.
struct GpuLightNode {
    // The bounds' middle; w half their diagonal.
    centre: vec4<f32>,
    // The emission axis; w the power.
    axis: vec4<f32>,
    cos_o: f32,
    sin_o: f32,
    cos_e: f32,
    distant: u32,
    // The children, LIGHT_LEAF for a leaf.
    left: u32,
    right: u32,
    // The node above, LIGHT_LEAF at the root.
    parent: u32,
    // A leaf's light (LightSet::order[first]).
    light: u32,
};

const LIGHT_LEAF: u32 = 0xFFFFFFFFu;

fn light_has(l: GpuLight, flag: u32) -> bool {
    return (l.flags & flag) != 0u;
}

// f32::signum and 1.0.copysign(x): the sign bit's, -0.0 counting as negative.
fn lt_sign(x: f32) -> f32 {
    return select(1.0, -1.0, (bitcast<u32>(x) & 0x80000000u) != 0u);
}

fn lt_finite(x: f32) -> bool {
    return (bitcast<u32>(x) & 0x7f800000u) != 0x7f800000u;
}

fn lt_normalized(v: vec3<f32>) -> vec3<f32> {
    return v * (1.0 / sqrt(dot(v, v)));
}

// a*b - c*d as the processor's f64 cross product rounds it: the products
// exact, then one rounding (Kahan's difference of products).
fn lt_cross_term(a: f32, b: f32, c: f32, d: f32) -> f32 {
    let cd = c * d;
    let error = fma(-c, d, cd);
    let term = fma(a, b, -cd);
    return term + error;
}

// AreaLight::spread_attenuation.
fn light_spread_attenuation(l: GpuLight, cos_light: f32) -> f32 {
    if (light_has(l, LF_SPREAD_FULL)) {
        return 1.0;
    }
    let c = clamp(cos_light, 1e-6, 1.0);
    let t = sqrt(max(1.0 - c * c, 0.0)) / c;
    return max((l.tan_half - t) * l.spread_norm, 0.0);
}

// AreaLight::face.
fn light_face(l: GpuLight, su: f32, sv: f32) -> f32 {
    if (light_has(l, LF_EVEN)) {
        return 1.0;
    }
    let falloff = clamp(l.falloff, 0.0, 1.0);
    let fade = clamp(l.fade, -1.0, 1.0);
    var edge: f32;
    var edge_mean: f32;
    if (light_has(l, LF_DISC)) {
        edge = 1.0 - falloff * min(su * su + sv * sv, 1.0);
        edge_mean = 1.0 - 0.5 * falloff;
    } else {
        let mean = 1.0 - falloff / 3.0;
        edge = (1.0 - falloff * su * su) * (1.0 - falloff * sv * sv);
        edge_mean = mean * mean;
    }
    let along = 0.5 * (1.0 - clamp(sv, -1.0, 1.0) * lt_sign(fade));
    let ramp = 1.0 - abs(fade) * along;
    return max(edge * ramp / (edge_mean * (1.0 - 0.5 * abs(fade))), 0.0);
}

// AreaLight::face_at.
fn light_face_at(l: GpuLight, point: vec3<f32>) -> f32 {
    if (light_has(l, LF_EVEN)) {
        return 1.0;
    }
    let offset = point - l.position.xyz;
    return light_face(l, dot(offset, l.half_u.xyz) / l.half_u.w, dot(offset, l.half_v.xyz) / l.half_v.w);
}

// SphericalRectangle [Urena et al. 2013] from `from`.
struct SphericalRectangle {
    ok: bool,
    z: vec3<f32>,
    z0: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    b0: f32,
    b1: f32,
    k: f32,
    solid_angle: f32,
};

fn spherical_rectangle(l: GpuLight, at: vec3<f32>) -> SphericalRectangle {
    var r: SphericalRectangle;
    r.ok = false;
    if (!light_has(l, LF_RECTANGLE)) {
        return r;
    }
    let d = l.rect_origin.xyz - at;
    var z = l.rect_z.xyz;
    var z0 = dot(d, z);
    if (z0 > 0.0) {
        z = -z;
        z0 = -z0;
    }
    if (abs(z0) < 1e-5) {
        return r;
    }
    let x0 = dot(d, l.rect_x.xyz);
    let y0 = dot(d, l.rect_y.xyz);
    let x1 = x0 + l.rect_x.w;
    let y1 = y0 + l.rect_y.w;
    // The corners' great circles: v00 x v10, v10 x v11, v11 x v01 and
    // v01 x v00, each with the one term that cancels exactly left nought.
    let n0 = lt_normalized(vec3<f32>(0.0, lt_cross_term(z0, x1, x0, z0), lt_cross_term(x0, y0, y0, x1)));
    let n1 = lt_normalized(vec3<f32>(lt_cross_term(y0, z0, z0, y1), 0.0, lt_cross_term(x1, y1, y0, x1)));
    let n2 = lt_normalized(vec3<f32>(0.0, lt_cross_term(z0, x0, x1, z0), lt_cross_term(x1, y1, y1, x0)));
    let n3 = lt_normalized(vec3<f32>(lt_cross_term(y1, z0, z0, y0), 0.0, lt_cross_term(x0, y0, y1, x0)));
    let g0 = acos(clamp(-dot(n0, n1), -1.0, 1.0));
    let g1 = acos(clamp(-dot(n1, n2), -1.0, 1.0));
    let g2 = acos(clamp(-dot(n2, n3), -1.0, 1.0));
    let g3 = acos(clamp(-dot(n3, n0), -1.0, 1.0));
    let solid_angle = g0 + g1 + g2 + g3 - LT_TAU;
    if (!lt_finite(solid_angle) || solid_angle < 1e-7) {
        return r;
    }
    return SphericalRectangle(true, z, z0, x0, y0, x1, y1, n0.z, n2.z, LT_TAU - g2 - g3, solid_angle);
}

// SphericalRectangle::direction.
fn spherical_rectangle_direction(l: GpuLight, r: SphericalRectangle, u: vec2<f32>) -> vec3<f32> {
    let au = u.x * r.solid_angle + r.k;
    let fu = (cos(au) * r.b0 - r.b1) / sin(au);
    let cu = clamp(lt_sign(fu) / sqrt(fu * fu + r.b0 * r.b0), -1.0, 1.0);
    let xu = clamp(-(cu * r.z0) / sqrt(max(1.0 - cu * cu, 1e-12)), r.x0, r.x1);
    let d = sqrt(xu * xu + r.z0 * r.z0);
    let h0 = r.y0 / sqrt(d * d + r.y0 * r.y0);
    let h1 = r.y1 / sqrt(d * d + r.y1 * r.y1);
    let hv = h0 + u.y * (h1 - h0);
    let hv2 = hv * hv;
    var yv = r.y1;
    if (hv2 < 1.0 - 1e-6) {
        yv = (hv * d) / sqrt(1.0 - hv2);
    }
    let offset = l.rect_x.xyz * xu + l.rect_y.xyz * yv + r.z * r.z0;
    let len2 = dot(offset, offset);
    if (len2 > 0.0 && lt_finite(len2)) {
        return offset * (1.0 / sqrt(len2));
    }
    return r.z;
}

// LightSample, with `infinite` for a distance that is.
struct LightDraw {
    ok: bool,
    direction: vec3<f32>,
    radiance: vec3<f32>,
    distance: f32,
    infinite: bool,
    pdf: f32,
    is_delta: bool,
};

fn no_light_draw() -> LightDraw {
    var d: LightDraw;
    d.ok = false;
    return d;
}

// AreaLight::finish: a point drawn on the face, by area.
fn area_finish(l: GpuLight, at: vec3<f32>, point: vec3<f32>, normal: vec3<f32>) -> LightDraw {
    let offset = point - at;
    let distance_squared = dot(offset, offset);
    if (distance_squared < 1e-12) {
        return no_light_draw();
    }
    let distance = sqrt(distance_squared);
    let direction = offset * (1.0 / distance);
    var cos_light = -dot(direction, normal);
    if (light_has(l, LF_TWO_SIDED)) {
        cos_light = abs(cos_light);
    }
    if (cos_light <= 1e-6) {
        return no_light_draw();
    }
    let pdf = distance_squared / (cos_light * max(l.normal.w, 1e-12));
    let spread = light_spread_attenuation(l, cos_light) * light_face_at(l, point);
    if (spread <= 0.0) {
        return no_light_draw();
    }
    return LightDraw(true, direction, l.radiance.xyz * spread, distance, false, pdf, false);
}

// AreaLight::sample.
fn area_sample(l: GpuLight, at: vec3<f32>, u: vec2<f32>) -> LightDraw {
    let normal = l.normal.xyz;
    let facing = dot(l.position.xyz - at, normal);
    if (!light_has(l, LF_TWO_SIDED) && facing > 0.0) {
        return no_light_draw();
    }
    var rectangle: SphericalRectangle;
    rectangle.ok = false;
    if (!light_has(l, LF_DISC)) {
        rectangle = spherical_rectangle(l, at);
    }
    if (rectangle.ok) {
        let direction = spherical_rectangle_direction(l, rectangle, u);
        var cos_light = -dot(direction, normal);
        if (light_has(l, LF_TWO_SIDED)) {
            cos_light = abs(cos_light);
        }
        if (cos_light <= 1e-6) {
            return no_light_draw();
        }
        // distance_along.
        let denominator = dot(direction, normal);
        if (abs(denominator) < 1e-9) {
            return no_light_draw();
        }
        let t = dot(l.position.xyz - at, normal) / denominator;
        if (!(t > 1e-6)) {
            return no_light_draw();
        }
        let spread = light_spread_attenuation(l, cos_light) * light_face_at(l, at + direction * t);
        if (spread <= 0.0) {
            return no_light_draw();
        }
        return LightDraw(true, direction, l.radiance.xyz * spread, t, false, 1.0 / rectangle.solid_angle, false);
    }
    var su: f32;
    var sv: f32;
    if (light_has(l, LF_DISC)) {
        let radius = sqrt(max(u.x, 0.0));
        let angle = LT_TAU * u.y;
        su = radius * cos(angle);
        sv = radius * sin(angle);
    } else {
        su = 2.0 * u.x - 1.0;
        sv = 2.0 * u.y - 1.0;
    }
    let point = l.position.xyz + l.half_u.xyz * su + l.half_v.xyz * sv;
    return area_finish(l, at, point, normal);
}

// SpotLight::falloff_at.
fn spot_falloff(l: GpuLight, cos_angle: f32) -> f32 {
    let cos_outer = l.cos_a;
    let cos_inner = l.cos_b;
    if (cos_angle <= cos_outer) {
        return 0.0;
    }
    if (cos_angle >= cos_inner) {
        return 1.0;
    }
    let t = clamp((cos_angle - cos_outer) / max(cos_inner - cos_outer, 1e-6), 0.0, 1.0);
    let smoothed = t * t * (3.0 - 2.0 * t);
    // powf, exactly where the exponent is one or nought.
    let exponent = max(l.exponent, 0.0);
    if (exponent == 1.0) {
        return smoothed;
    }
    if (exponent == 0.0) {
        return 1.0;
    }
    return pow(smoothed, exponent);
}

// Light::sample, by kind.
fn light_sample(l: GpuLight, at: vec3<f32>, u: vec2<f32>) -> LightDraw {
    if (!light_has(l, LF_ENABLED)) {
        return no_light_draw();
    }
    switch l.kind {
        case 0u: {
            return area_sample(l, at, u);
        }
        case 1u, 2u: {
            var offset = l.position.xyz - at;
            if (l.kind == LIGHT_POINT && l.position.w > 0.0) {
                // A point on the sphere's surface, facing the shading point.
                let away = at - l.position.xyz;
                let len2 = dot(away, away);
                if (!(len2 > 0.0 && lt_finite(len2))) {
                    return no_light_draw();
                }
                let basis = frame_of(away * (1.0 / sqrt(len2)));
                let z = 1.0 - 2.0 * u.x;
                let r = sqrt(max(1.0 - z * z, 0.0));
                let phi = LT_TAU * u.y;
                offset = l.position.xyz + to_world(basis, vec3<f32>(r * cos(phi), r * sin(phi), z)) * l.position.w - at;
            }
            let distance_squared = dot(offset, offset);
            if (distance_squared < 1e-12) {
                return no_light_draw();
            }
            let distance = sqrt(distance_squared);
            let direction = offset * (1.0 / distance);
            if (l.kind == LIGHT_POINT) {
                return LightDraw(true, direction, l.radiance.xyz * (1.0 / distance_squared), distance, false, 1.0, !(l.position.w > 0.0));
            }
            let falloff = spot_falloff(l, dot(-direction, l.normal.xyz));
            if (falloff <= 0.0) {
                return no_light_draw();
            }
            return LightDraw(true, direction, l.radiance.xyz * falloff * (1.0 / distance_squared), distance, false, 1.0, true);
        }
        case 4u: {
            let towards = -l.normal.xyz;
            if (light_has(l, LF_DELTA)) {
                return LightDraw(true, towards, l.radiance.xyz, 0.0, true, 1.0, true);
            }
            let cos_max = l.cos_a;
            let c = 1.0 - u.x * (1.0 - cos_max);
            let s = sqrt(max(1.0 - c * c, 0.0));
            let phi = LT_TAU * u.y;
            let direction = to_world(frame_of(towards), vec3<f32>(s * cos(phi), s * sin(phi), c));
            return LightDraw(true, direction, l.radiance.xyz * l.radiance.w, 0.0, true, l.exponent, false);
        }
        default: {
            return no_light_draw();
        }
    }
}

// What a ray meets of a lamp (Light::met_along): the radiance it sees and
// how far along; a sun's is infinitely far.
struct LightMet {
    ok: bool,
    radiance: vec3<f32>,
    t: f32,
    infinite: bool,
};

fn light_met_along(l: GpuLight, at: vec3<f32>, direction: vec3<f32>) -> LightMet {
    var none: LightMet;
    none.ok = false;
    if (!light_has(l, LF_ENABLED)) {
        return none;
    }
    if (l.kind == LIGHT_AREA) {
        let normal = l.normal.xyz;
        let denominator = dot(direction, normal);
        if (abs(denominator) < 1e-6) {
            return none;
        }
        let t = dot(l.position.xyz - at, normal) / denominator;
        if (!(t > 1e-6)) {
            return none;
        }
        let offset = at + direction * t - l.position.xyz;
        let su = dot(offset, l.half_u.xyz) / l.half_u.w;
        let sv = dot(offset, l.half_v.xyz) / l.half_v.w;
        var inside: bool;
        if (light_has(l, LF_DISC)) {
            inside = su * su + sv * sv <= 1.0;
        } else {
            inside = abs(su) <= 1.0 && abs(sv) <= 1.0;
        }
        let cos_light = select(-denominator, abs(denominator), light_has(l, LF_TWO_SIDED));
        if (!inside || cos_light <= 1e-6) {
            return none;
        }
        let spread = light_spread_attenuation(l, cos_light) * light_face(l, su, sv);
        if (!(spread > 0.0)) {
            return none;
        }
        return LightMet(true, l.radiance.xyz * spread, t, false);
    }
    if (l.kind == LIGHT_DISTANT) {
        if (light_has(l, LF_DELTA) || dot(-l.normal.xyz, direction) < l.cos_a) {
            return none;
        }
        return LightMet(true, l.radiance.xyz * l.radiance.w, 0.0, true);
    }
    return none;
}

// Light::density_along: AreaLight::pdf, a sun's over its disc, else none.
fn light_density_along(l: GpuLight, at: vec3<f32>, direction: vec3<f32>) -> f32 {
    if (l.kind == LIGHT_AREA) {
        let normal = l.normal.xyz;
        let denominator = dot(direction, normal);
        if (abs(denominator) < 1e-6) {
            return 0.0;
        }
        let t = dot(l.position.xyz - at, normal) / denominator;
        if (t <= 1e-6) {
            return 0.0;
        }
        let offset = at + direction * t - l.position.xyz;
        let su = dot(offset, l.half_u.xyz) / l.half_u.w;
        let sv = dot(offset, l.half_v.xyz) / l.half_v.w;
        var inside: bool;
        if (light_has(l, LF_DISC)) {
            inside = su * su + sv * sv <= 1.0;
        } else {
            inside = abs(su) <= 1.0 && abs(sv) <= 1.0;
        }
        if (!inside) {
            return 0.0;
        }
        let cos_light = select(-denominator, abs(denominator), light_has(l, LF_TWO_SIDED));
        if (cos_light <= 1e-6) {
            return 0.0;
        }
        if (!light_has(l, LF_DISC)) {
            let rectangle = spherical_rectangle(l, at);
            if (rectangle.ok) {
                return 1.0 / rectangle.solid_angle;
            }
        }
        return (t * t) / (cos_light * max(l.normal.w, 1e-12));
    }
    if (l.kind == LIGHT_DISTANT) {
        if (light_has(l, LF_DELTA) || dot(-l.normal.xyz, direction) < l.cos_a) {
            return 0.0;
        }
        return l.exponent;
    }
    return 0.0;
}

// LightNode::importance: how much a subtree is worth sampling from `point`,
// by cosines as the processor works it.
fn light_node_importance(n: GpuLightNode, point: vec3<f32>, normal: vec3<f32>, two_sided: bool) -> f32 {
    let power = n.axis.w;
    if (power <= 0.0) {
        return 0.0;
    }
    var direction: vec3<f32>;
    var distance_squared: f32;
    var cos_u: f32;
    var sin_u: f32;
    let distant = n.distant != 0u;
    if (distant) {
        direction = -n.axis.xyz;
        distance_squared = 1.0;
        cos_u = n.cos_o;
        sin_u = n.sin_o;
    } else {
        let half = n.centre.w;
        let offset = n.centre.xyz - point;
        let distance = sqrt(dot(offset, offset));
        direction = select(normal, offset * (1.0 / distance), distance > 1e-6);
        if (distance > half) {
            sin_u = clamp(half / distance, 0.0, 1.0);
            cos_u = sqrt(max(1.0 - sin_u * sin_u, 0.0));
        } else {
            cos_u = -1.0;
            sin_u = 0.0;
        }
        let kept = max(max(distance, 0.5 * half), 1e-4);
        distance_squared = kept * kept;
    }
    var cos_i = clamp(dot(direction, normal), -1.0, 1.0);
    if (two_sided) {
        cos_i = abs(cos_i);
    }
    var cos_incident = 1.0;
    if (!(cos_i >= cos_u)) {
        cos_incident = cos_i * cos_u + sqrt(max(1.0 - cos_i * cos_i, 0.0)) * sin_u;
    }
    if (cos_incident <= 0.0) {
        return 0.0;
    }
    var outgoing = 1.0;
    if (!distant) {
        let cos_out = clamp(dot(-direction, n.axis.xyz), -1.0, 1.0);
        let cos_ou = n.cos_o * cos_u - n.sin_o * sin_u;
        if (!(cos_u <= -n.cos_o || cos_out >= cos_ou)) {
            let sin_ou = n.sin_o * cos_u + n.cos_o * sin_u;
            let outside = cos_out * cos_ou + sqrt(max(1.0 - cos_out * cos_out, 0.0)) * sin_ou;
            if (outside <= n.cos_e) {
                return 0.0;
            }
            outgoing = outside;
        }
    }
    return power * cos_incident * outgoing / distance_squared;
}
