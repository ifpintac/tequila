// The per-pixel sampler: tequila_core::sampler, bit for bit.
//
// The processor's sampler keeps each dimension group's shuffled index once
// worked out; that only saves time, and the numbers are the same worked out
// each draw, which is what this does.
//#include sobol

const DIMENSIONS_PER_BOUNCE: u32 = 8u;
const FIRST_BOUNCE_DIMENSION: u32 = 8u;
const SOBOL_DIMENSION_LIMIT: u32 = 64u;

// sampler::dim
const DIM_PIXEL: u32 = 0u;
const DIM_LENS: u32 = 2u;
const DIM_TIME: u32 = 4u;
const DIM_WAVELENGTH: u32 = 5u;
const DIM_BSDF_DIRECTION: u32 = 0u;
const DIM_BSDF_LOBE: u32 = 2u;
const DIM_RUSSIAN_ROULETTE: u32 = 3u;
const DIM_LIGHT_SAMPLE: u32 = 4u;
const DIM_LIGHT_SELECT: u32 = 6u;
const DIM_MEDIUM: u32 = 7u;

struct Sampler {
    seed: u32,
    sample_index: u32,
    bounce_base: u32,
    fallback: Pcg32,
};

fn sampler_new(pixel_x: u32, pixel_y: u32, sample_index: u32, scene_seed: u32) -> Sampler {
    let seed = hash3(scene_seed, pixel_x, pixel_y);
    return Sampler(
        seed,
        sample_index,
        FIRST_BOUNCE_DIMENSION,
        pcg32_new(hash3(seed, sample_index, 0x5f3du), hash3(seed, sample_index, 0xa17cu)),
    );
}

fn sampler_begin_bounce(s: ptr<function, Sampler>, bounce: u32) {
    (*s).bounce_base = FIRST_BOUNCE_DIMENSION + bounce * DIMENSIONS_PER_BOUNCE;
}

// The raw 32 bits behind sampler_get: a scrambled Sobol' value below the
// Sobol' limit, the next PCG32 draw past it.
fn sampler_get_u32(s: ptr<function, Sampler>, dimension: u32) -> u32 {
    if (dimension < SOBOL_DIMENSION_LIMIT) {
        return sobol_sample_u32((*s).sample_index, dimension, (*s).seed);
    }
    var fallback = (*s).fallback;
    let x = pcg32_step(&fallback);
    (*s).fallback = fallback;
    return x;
}

fn sampler_get(s: ptr<function, Sampler>, dimension: u32) -> f32 {
    return to_unit(sampler_get_u32(s, dimension));
}

fn sampler_get2(s: ptr<function, Sampler>, dimension: u32) -> vec2<f32> {
    let a = sampler_get(s, dimension);
    let b = sampler_get(s, dimension + 1u);
    return vec2<f32>(a, b);
}

fn sampler_pixel_jitter(s: ptr<function, Sampler>) -> vec2<f32> { return sampler_get2(s, DIM_PIXEL); }
fn sampler_lens(s: ptr<function, Sampler>) -> vec2<f32> { return sampler_get2(s, DIM_LENS); }
fn sampler_time(s: ptr<function, Sampler>) -> f32 { return sampler_get(s, DIM_TIME); }
fn sampler_wavelength(s: ptr<function, Sampler>) -> f32 { return sampler_get(s, DIM_WAVELENGTH); }
fn sampler_light_select(s: ptr<function, Sampler>) -> f32 { return sampler_get(s, (*s).bounce_base + DIM_LIGHT_SELECT); }
fn sampler_light_sample(s: ptr<function, Sampler>) -> vec2<f32> { return sampler_get2(s, (*s).bounce_base + DIM_LIGHT_SAMPLE); }
fn sampler_bsdf_lobe(s: ptr<function, Sampler>) -> f32 { return sampler_get(s, (*s).bounce_base + DIM_BSDF_LOBE); }
fn sampler_bsdf_direction(s: ptr<function, Sampler>) -> vec2<f32> { return sampler_get2(s, (*s).bounce_base + DIM_BSDF_DIRECTION); }
fn sampler_russian_roulette(s: ptr<function, Sampler>) -> f32 { return sampler_get(s, (*s).bounce_base + DIM_RUSSIAN_ROULETTE); }
fn sampler_medium(s: ptr<function, Sampler>) -> f32 { return sampler_get(s, (*s).bounce_base + DIM_MEDIUM); }
fn sampler_next_unstructured(s: ptr<function, Sampler>) -> f32 {
    var fallback = (*s).fallback;
    let x = to_unit(pcg32_step(&fallback));
    (*s).fallback = fallback;
    return x;
}
