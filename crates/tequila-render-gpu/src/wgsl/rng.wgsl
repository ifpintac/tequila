// Hashes and the PCG32 generator: tequila_core::rng, bit for bit.
//
// WGSL has no 64-bit integers, and PCG32's state is one: here it is two
// 32-bit words, low then high, and the multiply and add are done in halves.

fn hash_u32(v: u32) -> u32 {
    var x = v;
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

fn hash_combine(seed: u32, v: u32) -> u32 {
    return hash_u32(seed ^ (v * 0x9e3779b9u));
}

// hash_many over three values, the most any caller here passes.
fn hash3(a: u32, b: u32, c: u32) -> u32 {
    return hash_combine(hash_combine(hash_combine(0x811c9dc5u, a), b), c);
}

// The high 32 bits of a 32 x 32 multiply.
fn mul_hi(a: u32, b: u32) -> u32 {
    let a_lo = a & 0xffffu;
    let a_hi = a >> 16u;
    let b_lo = b & 0xffffu;
    let b_hi = b >> 16u;
    let lo_lo = a_lo * b_lo;
    let hi_lo = a_hi * b_lo;
    let lo_hi = a_lo * b_hi;
    let hi_hi = a_hi * b_hi;
    let cross = (lo_lo >> 16u) + (hi_lo & 0xffffu) + (lo_hi & 0xffffu);
    return hi_hi + (hi_lo >> 16u) + (lo_hi >> 16u) + (cross >> 16u);
}

// 64-bit values as (low, high).
fn mul64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    return vec2<u32>(a.x * b.x, mul_hi(a.x, b.x) + a.x * b.y + a.y * b.x);
}

fn add64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.x + b.x;
    return vec2<u32>(lo, a.y + b.y + select(0u, 1u, lo < a.x));
}

const PCG32_MULT: vec2<u32> = vec2<u32>(0x4c957f2du, 0x5851f42du);

struct Pcg32 {
    state: vec2<u32>,
    inc: vec2<u32>,
};

fn pcg32_step(r: ptr<function, Pcg32>) -> u32 {
    let old = (*r).state;
    (*r).state = add64(mul64(old, PCG32_MULT), (*r).inc);
    // ((old >> 18) ^ old) >> 27, as 32 bits.
    let s_lo = (old.x >> 18u) | (old.y << 14u);
    let s_hi = old.y >> 18u;
    let x_lo = s_lo ^ old.x;
    let x_hi = s_hi ^ old.y;
    let xorshifted = (x_lo >> 27u) | (x_hi << 5u);
    let rot = old.y >> 27u;
    return (xorshifted >> rot) | (xorshifted << ((32u - rot) & 31u));
}

// Pcg32::new(seq, seed) for a seq and seed that fit in 32 bits, as every
// caller's hashes do.
fn pcg32_new(seq: u32, seed: u32) -> Pcg32 {
    var r: Pcg32;
    r.state = vec2<u32>(0u, 0u);
    r.inc = vec2<u32>((seq << 1u) | 1u, seq >> 31u);
    _ = pcg32_step(&r);
    r.state = add64(r.state, vec2<u32>(seed, 0u));
    _ = pcg32_step(&r);
    return r;
}

// [0, 1) from the top 24 bits, never rounding up to one.
fn to_unit(x: u32) -> f32 {
    return min(f32(x >> 8u) * (1.0 / 16777216.0), bitcast<f32>(0x3f7fffffu));
}
