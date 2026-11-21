// Spectral paths: tequila_render_core::spectral, a path carrying four
// wavelengths rather than three channels (Wilkie et al.'s hero and its
// three companions, spaced evenly in the sampling distribution's measure).
// Every colour the path meets - a reflectance, a light - is read at the
// four as the smooth spectrum its RGB stands for, by Jakob and Hanika's
// sigmoid polynomials from the processor's own coefficient table; what the
// path carries goes to the film through the response the processor's
// Wavelengths weigh each wavelength by.
//
// The tables ride at the end of the pictures' texels (binding 16, no
// binding of their own), from the word spectral_base (the frame's), as
// transport.rs's SpectralTables lays them out: the film's response at 81
// wavelengths 5 nm apart (three floats each), the sampling distribution's
// 401 edges as the least float at or above each (for the search) and as a
// high and low float (for the place in the bin), its 400 bins' densities,
// the table's 64 brightnesses, and its 3 x 64^3 coefficient triples.
//
// Products an add reads go through cy_exact, quotients through cy_div and
// square roots cy_sqrt, so each rounds as Rust rounds it.
//#include features
//#include textures

const SPECTRAL_SHORTEST: f32 = 380.0;
const SPECTRAL_LONGEST: f32 = 780.0;
const SPECTRAL_RESOLUTION: u32 = 64u;
const SPECTRAL_BINS: u32 = 400u;
// The tables' places, in words from frame.spectral_base.
const SPEC_RESPONSE: u32 = 0u;
const SPEC_CDF_UP: u32 = 243u;
const SPEC_CDF_HIGH: u32 = 644u;
const SPEC_CDF_LOW: u32 = 1045u;
const SPEC_DENSITY: u32 = 1446u;
const SPEC_SCALE: u32 = 1846u;
const SPEC_COEFFICIENTS: u32 = 1910u;
// unbounded's brightness: a light's RGB brought to half at its largest.
const SPECTRAL_UNBOUNDED_BRIGHTNESS: f32 = 0.5;

// Where the tables start in texture_texels: the frame's spectral_base,
// set at a sample's start.
var<private> spectral_base: u32;

fn spec_word(k: u32) -> f32 {
    return bitcast<f32>(texture_texels[spectral_base + k]);
}

// The four wavelengths of the camera sample being traced (Wavelengths):
// the hero first; the same, normalized over the range, for the sigmoids;
// and what one unit of radiance at each adds to the film's red, green and
// blue (a channel a vector, a wavelength a lane).
var<private> spectral_lambda: vec4<f32>;
var<private> spectral_u: vec4<f32>;
var<private> spectral_film_r: vec4<f32>;
var<private> spectral_film_g: vec4<f32>;
var<private> spectral_film_b: vec4<f32>;

// Response::at: the film's response at `wavelength`, between the nodes.
fn spectral_response(wavelength: f32) -> vec3<f32> {
    let at = clamp(cy_div(wavelength - SPECTRAL_SHORTEST, 5.0), 0.0, 80.0);
    let i = min(u32(at), 79u);
    let t = at - f32(i);
    let a = vec3<f32>(spec_word(SPEC_RESPONSE + 3u * i), spec_word(SPEC_RESPONSE + 3u * i + 1u), spec_word(SPEC_RESPONSE + 3u * i + 2u));
    let b = vec3<f32>(spec_word(SPEC_RESPONSE + 3u * i + 3u), spec_word(SPEC_RESPONSE + 3u * i + 4u), spec_word(SPEC_RESPONSE + 3u * i + 5u));
    return a + cy_exact3((b - a) * t);
}

// sample_with_density: a wavelength from `u` in [0, 1) by inverting the
// distribution's tabulated CDF, and the density of the bin it fell in. The
// bin is found exactly (an f32 is at least an f64 edge exactly when it is
// at least the least f32 at or above that edge); the place in it from the
// edge as two floats.
fn spectral_sample(u_in: f32) -> vec2<f32> {
    let u = clamp(u_in, 0.0, 1.0);
    // partition_point(c <= u) - 1: the last edge at or below u, of 401.
    var lo = 0u;
    var hi = SPECTRAL_BINS + 1u;
    loop {
        if (lo >= hi) {
            break;
        }
        let mid = (lo + hi) / 2u;
        if (spec_word(SPEC_CDF_UP + mid) <= u) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    let bin = min(max(lo, 1u) - 1u, SPECTRAL_BINS - 1u);
    let width = (spec_word(SPEC_CDF_HIGH + bin + 1u) - spec_word(SPEC_CDF_HIGH + bin)) + (spec_word(SPEC_CDF_LOW + bin + 1u) - spec_word(SPEC_CDF_LOW + bin));
    var t = 0.5;
    if (width > 0.0) {
        let d = (u - spec_word(SPEC_CDF_HIGH + bin)) - spec_word(SPEC_CDF_LOW + bin);
        t = clamp(cy_div(d, width), 0.0, 1.0);
    }
    return vec2<f32>((SPECTRAL_SHORTEST + f32(bin)) + t, spec_word(SPEC_DENSITY + bin));
}

// Wavelengths::new: the four from `u`, the hero at `u`, the others a
// quarter, a half and three quarters further round.
fn spectral_begin(u: f32) {
    var lambda = vec4<f32>(0.0);
    var r = vec4<f32>(0.0);
    var g = vec4<f32>(0.0);
    var b = vec4<f32>(0.0);
    for (var i = 0u; i < 4u; i++) {
        let v = u + f32(i) * 0.25;
        let ui = v - trunc(v);
        let drawn = spectral_sample(ui);
        let response = spectral_response(drawn.x);
        let weight = cy_div(1.0, 4.0 * max(drawn.y, 1e-12));
        let lane = vec4<u32>(0u, 1u, 2u, 3u) == vec4<u32>(i);
        lambda = select(lambda, vec4<f32>(drawn.x), lane);
        r = select(r, vec4<f32>(cy_exact(response.x * weight)), lane);
        g = select(g, vec4<f32>(cy_exact(response.y * weight)), lane);
        b = select(b, vec4<f32>(cy_exact(response.z * weight)), lane);
    }
    spectral_lambda = lambda;
    spectral_u = vec4<f32>(
        cy_div(lambda.x - SPECTRAL_SHORTEST, SPECTRAL_LONGEST - SPECTRAL_SHORTEST),
        cy_div(lambda.y - SPECTRAL_SHORTEST, SPECTRAL_LONGEST - SPECTRAL_SHORTEST),
        cy_div(lambda.z - SPECTRAL_SHORTEST, SPECTRAL_LONGEST - SPECTRAL_SHORTEST),
        cy_div(lambda.w - SPECTRAL_SHORTEST, SPECTRAL_LONGEST - SPECTRAL_SHORTEST),
    );
    spectral_film_r = r;
    spectral_film_g = g;
    spectral_film_b = b;
}

// sigmoid: 0.5 + x / (2 sqrt(1 + x^2)), 0 and 1 at the infinities.
fn spectral_sigmoid(x: f32) -> f32 {
    if (abs(x) > 3.40282347e38) {
        return select(0.0, 1.0, x > 0.0);
    }
    return 0.5 + cy_div(cy_exact(0.5 * x), cy_sqrt(1.0 + cy_exact(x * x)));
}

// Sigmoid::at, at each of the four wavelengths.
fn spectral_at(c: vec3<f32>) -> vec4<f32> {
    if (abs(c.z) > 3.40282347e38) {
        return vec4<f32>(spectral_sigmoid(c.z));
    }
    var out = vec4<f32>(0.0);
    let u = spectral_u;
    out.x = spectral_sigmoid(cy_exact(cy_exact(cy_exact(c.x * u.x) + c.y) * u.x) + c.z);
    out.y = spectral_sigmoid(cy_exact(cy_exact(cy_exact(c.x * u.y) + c.y) * u.y) + c.z);
    out.z = spectral_sigmoid(cy_exact(cy_exact(cy_exact(c.x * u.z) + c.y) * u.z) + c.z);
    out.w = spectral_sigmoid(cy_exact(cy_exact(cy_exact(c.x * u.w) + c.y) * u.w) + c.z);
    return out;
}

// Sigmoid::constant's c2: the flat spectrum of `v`.
fn spectral_constant(v: f32) -> vec3<f32> {
    if (v <= 0.0) {
        return vec3<f32>(0.0, 0.0, -bitcast<f32>(0x7F800000u));
    }
    if (v >= 1.0) {
        return vec3<f32>(0.0, 0.0, bitcast<f32>(0x7F800000u));
    }
    return vec3<f32>(0.0, 0.0, cy_div(v - 0.5, cy_sqrt(cy_exact(v * (1.0 - v)))));
}

fn spectral_coefficients(largest: u32, k: u32, j: u32, i: u32) -> vec3<f32> {
    let at = SPEC_COEFFICIENTS + 3u * (((largest * SPECTRAL_RESOLUTION + k) * SPECTRAL_RESOLUTION + j) * SPECTRAL_RESOLUTION + i);
    return vec3<f32>(spec_word(at), spec_word(at + 1u), spec_word(at + 2u));
}

// Table::sigmoid: the polynomial whose colour under D65 is `rgb_in`, by
// trilinear interpolation in the cube its largest channel picks.
fn spectral_sigmoid_of(rgb_in: vec3<f32>) -> vec3<f32> {
    let rgb = clamp(rgb_in, vec3<f32>(0.0), vec3<f32>(1.0));
    if (rgb.x == rgb.y && rgb.y == rgb.z) {
        return spectral_constant(rgb.x);
    }
    var largest = 2u;
    if (rgb.x > rgb.y) {
        if (rgb.x > rgb.z) {
            largest = 0u;
        }
    } else if (rgb.y > rgb.z) {
        largest = 1u;
    }
    let z = rgb[largest];
    let steps = f32(SPECTRAL_RESOLUTION - 1u);
    let x = cy_div(cy_exact(rgb[(largest + 1u) % 3u] * steps), z);
    let y = cy_div(cy_exact(rgb[(largest + 2u) % 3u] * steps), z);
    let xi = min(u32(x), SPECTRAL_RESOLUTION - 2u);
    let yi = min(u32(y), SPECTRAL_RESOLUTION - 2u);
    // The last brightness at or below z among the 62 inner ones.
    var zi = 0u;
    for (var s = 1u; s < SPECTRAL_RESOLUTION - 1u; s++) {
        if (spec_word(SPEC_SCALE + s) <= z) {
            zi = s;
        }
    }
    let dx = x - f32(xi);
    let dy = y - f32(yi);
    let s0 = spec_word(SPEC_SCALE + zi);
    let s1 = spec_word(SPEC_SCALE + zi + 1u);
    let dz = clamp(cy_div(z - s0, s1 - s0), 0.0, 1.0);
    var c = vec3<f32>(0.0);
    for (var n = 0u; n < 8u; n++) {
        let kk = n >> 2u;
        let jj = (n >> 1u) & 1u;
        let ii = n & 1u;
        let wz = select(1.0 - dz, dz, kk == 1u);
        let wy = select(1.0 - dy, dy, jj == 1u);
        let wx = select(1.0 - dx, dx, ii == 1u);
        let w = cy_exact(cy_exact(wz * wy) * wx);
        c = c + cy_exact3(spectral_coefficients(largest, zi + kk, yi + jj, xi + ii) * w);
    }
    return c;
}

// Wavelengths::reflectance: a reflectance, transmittance or weight at the
// four wavelengths; a grey exactly flat, a weight above one as an
// unbounded colour.
fn spectral_reflectance(rgb: vec3<f32>) -> vec4<f32> {
    if (rgb.x == rgb.y && rgb.y == rgb.z) {
        return vec4<f32>(rgb.x);
    }
    if (max(max(rgb.x, rgb.y), rgb.z) > 1.0) {
        return spectral_unbounded(rgb);
    }
    return spectral_at(spectral_sigmoid_of(rgb));
}

// Wavelengths::unbounded: a light, or anything with no ceiling, brought to
// half at its largest channel, looked up there and scaled back.
fn spectral_unbounded(rgb: vec3<f32>) -> vec4<f32> {
    if (rgb.x == rgb.y && rgb.y == rgb.z) {
        return vec4<f32>(rgb.x);
    }
    let largest = max(max(rgb.x, rgb.y), rgb.z);
    if (largest <= 0.0) {
        return vec4<f32>(0.0);
    }
    let scale = cy_div(largest, SPECTRAL_UNBOUNDED_BRIGHTNESS);
    let inverse = cy_div(1.0, scale);
    return cy_exact4(spectral_at(spectral_sigmoid_of(cy_exact3(rgb * inverse))) * scale);
}

fn cy_exact4(v: vec4<f32>) -> vec4<f32> {
    return bitcast<vec4<f32>>(bitcast<vec4<u32>>(v) | vec4<u32>(cy_zero));
}

// Wavelengths::to_rgb: what the four values add to the film.
fn spectral_rgb(v: vec4<f32>) -> vec3<f32> {
    var out = vec3<f32>(0.0);
    out = out + cy_exact3(vec3<f32>(spectral_film_r.x, spectral_film_g.x, spectral_film_b.x) * v.x);
    out = out + cy_exact3(vec3<f32>(spectral_film_r.y, spectral_film_g.y, spectral_film_b.y) * v.y);
    out = out + cy_exact3(vec3<f32>(spectral_film_r.z, spectral_film_g.z, spectral_film_b.z) * v.z);
    out = out + cy_exact3(vec3<f32>(spectral_film_r.w, spectral_film_g.w, spectral_film_b.w) * v.w);
    return out;
}

// hero_alone: the hero alone, weighted for the three it leaves behind.
fn spectral_hero_alone(v: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(v.x * 4.0, 0.0, 0.0, 0.0);
}

// Wavelengths::luminance: how luminous the film finds the path's light per
// unit - near 1 for all four, the hero's own once it goes on alone.
fn spectral_hue(alone: bool) -> f32 {
    if (alone) {
        let hero = vec3<f32>(spectral_film_r.x, spectral_film_g.x, spectral_film_b.x) * 4.0;
        return cy_exact(cy_exact(0.2722287 * hero.x) + cy_exact(0.6740818 * hero.y)) + cy_exact(0.0536895 * hero.z);
    }
    let one = spectral_rgb(vec4<f32>(1.0));
    return cy_exact(cy_exact(0.2722287 * one.x) + cy_exact(0.6740818 * one.y)) + cy_exact(0.0536895 * one.z);
}
