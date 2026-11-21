// Thin-film interference over a specular interface:
// tequila_materials::thin_film, the processor's three-wavelength fallback -
// the Airy summation of every bounce inside a film at one wavelength per
// channel (600, 550 and 450 nm), in amplitudes, the two polarizations kept
// apart until their intensities are averaged, over a dielectric base or a
// conductor's complex index (n - ik); and Film's blend of that with the
// bare interface by the film's weight. A film is a vec4: its weight, its
// thickness in nanometres and its index, the fourth unused; a weight at or
// below 1e-4 or no thickness is no film (Film::is_negligible).
//
// Every product an add reads goes through cy_exact and every quotient
// through cy_div, so each rounds as Rust rounds it; the phase's sine and
// cosine are cy_sincos's, an ulp or so from the processor's.
//#include features
//#include nodes

// thin_film.rs's WAVELENGTHS, one a channel.
const FILM_WAVELENGTHS: vec3<f32> = vec3<f32>(600.0, 550.0, 450.0);
// 4 pi as an f32 (4.0 * PI, exactly).
const FILM_FOUR_PI: f32 = 12.566370964050293;

// Complex numbers as (re, im): thin_film.rs's Complex.
fn film_mul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(cy_exact(a.x * b.x) - cy_exact(a.y * b.y), cy_exact(a.x * b.y) + cy_exact(a.y * b.x));
}

fn film_div(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let denominator = cy_exact(b.x * b.x) + cy_exact(b.y * b.y);
    if (denominator <= 0.0) {
        return vec2<f32>(0.0);
    }
    return vec2<f32>(
        cy_div(cy_exact(a.x * b.x) + cy_exact(a.y * b.y), denominator),
        cy_div(cy_exact(a.y * b.x) - cy_exact(a.x * b.y), denominator),
    );
}

fn film_norm_squared(a: vec2<f32>) -> f32 {
    return cy_exact(a.x * a.x) + cy_exact(a.y * a.y);
}

// The principal square root.
fn film_sqrt(a: vec2<f32>) -> vec2<f32> {
    if (a.y == 0.0 && a.x >= 0.0) {
        return vec2<f32>(cy_sqrt(a.x), 0.0);
    }
    let modulus = cy_sqrt(cy_exact(a.x * a.x) + cy_exact(a.y * a.y));
    let re = cy_sqrt(max(0.5 * (modulus + a.x), 0.0));
    let im = cy_sqrt(max(0.5 * (modulus - a.x), 0.0));
    return vec2<f32>(re, select(im, -im, a.y < 0.0));
}

// interface: the amplitude reflections at an interface for the two
// polarizations, (s, p) as (s.re, s.im, p.re, p.im).
fn film_interface(eta_i: vec2<f32>, cos_i: vec2<f32>, eta_t: vec2<f32>, cos_t: vec2<f32>) -> vec4<f32> {
    let ei_ci = film_mul(eta_i, cos_i);
    let et_ct = film_mul(eta_t, cos_t);
    let et_ci = film_mul(eta_t, cos_i);
    let ei_ct = film_mul(eta_i, cos_t);
    let s = film_div(ei_ci - et_ct, ei_ci + et_ct);
    let p = film_div(et_ci - ei_ct, et_ci + ei_ct);
    return vec4<f32>(s, p);
}

// refracted_cosine: cos(theta) beyond an interface, by Snell's law.
fn film_refracted_cosine(eta_i: vec2<f32>, sin_i: f32, eta_t: vec2<f32>) -> vec2<f32> {
    let sin_t = film_div(film_mul(vec2<f32>(sin_i, 0.0), eta_i), eta_t);
    return film_sqrt(vec2<f32>(1.0, 0.0) - film_mul(sin_t, sin_t));
}

// combine: the film's whole series of bounces for one polarization.
fn film_combine(r_top: vec2<f32>, r_bottom: vec2<f32>, shift: vec2<f32>) -> f32 {
    let delayed = film_mul(r_bottom, shift);
    let numerator = r_top + delayed;
    let denominator = vec2<f32>(1.0, 0.0) + film_mul(r_top, delayed);
    return film_norm_squared(film_div(numerator, denominator));
}

// airy: the reflectance of a film `thickness` nm thick of index `film_ior`
// on a base of complex index `base`, seen from air at `cos_theta`, at one
// wavelength.
fn film_airy(cos_in: f32, film_ior: f32, base: vec2<f32>, thickness: f32, wavelength: f32) -> f32 {
    let cos_theta = clamp(cos_in, 0.0, 1.0);
    let sin_theta = cy_sqrt(max(1.0 - cy_exact(cos_theta * cos_theta), 0.0));
    let air = vec2<f32>(1.0, 0.0);
    let film = vec2<f32>(max(film_ior, 1.0), 0.0);
    let cos_film = film_refracted_cosine(air, sin_theta, film);
    let top = film_interface(air, vec2<f32>(cos_theta, 0.0), film, cos_film);
    // Snell into the base from air, not from inside the film.
    let cos_base = film_refracted_cosine(air, sin_theta, base);
    let bottom = film_interface(film, cos_film, base, cos_base);
    let phase = cy_div(cy_exact(cy_exact(cy_exact(FILM_FOUR_PI * film_ior) * thickness) * cos_film.x), wavelength);
    let turn = cy_sincos(-phase);
    let shift = vec2<f32>(turn.y, turn.x);
    return clamp(0.5 * (film_combine(top.xy, bottom.xy, shift) + film_combine(top.zw, bottom.zw, shift)), 0.0, 1.0);
}

// Whether `film` does anything (not Film::is_negligible).
fn film_on(film: vec4<f32>) -> bool {
    return film.x > 1e-4 && film.y > 0.0;
}

// Film::over_dielectric and ::over_conductor: `bare`, the interface's own
// reflectance at `cos_theta`, blended by the film's weight towards the
// coated one over a base of index `base_re` - i `base_im` per channel
// (a dielectric's index and nought, or a conductor's n and -k). Each
// channel's Airy sum from one place.
fn film_over(bare: vec3<f32>, cos_theta: f32, film: vec4<f32>, base_re: vec3<f32>, base_im: vec3<f32>) -> vec3<f32> {
    var coated = vec3<f32>(0.0);
    for (var c = 0u; c < 3u; c++) {
        let r = film_airy(cos_theta, film.z, vec2<f32>(base_re[c], base_im[c]), film.y, FILM_WAVELENGTHS[c]);
        coated = select(coated, vec3<f32>(r), vec3<u32>(0u, 1u, 2u) == vec3<u32>(c));
    }
    let w = clamp(film.x, 0.0, 1.0);
    return cy_exact3(bare * (1.0 - w)) + cy_exact3(coated * w);
}
