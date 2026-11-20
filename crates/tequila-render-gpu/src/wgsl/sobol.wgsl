// Owen-scrambled Sobol': tequila_core::sobol, bit for bit, reading the
// processor's own byte tables (tables.rs, @group(1) @binding(0)).
//#include rng

const NUM_BASE_DIMENSIONS: u32 = 4u;

@group(1) @binding(0) var<storage, read> sobol_tables: array<u32>;

fn sobol_u32(index: u32, dimension: u32) -> u32 {
    let base = dimension * 1024u;
    return sobol_tables[base + (index & 0xffu)]
        ^ sobol_tables[base + 256u + ((index >> 8u) & 0xffu)]
        ^ sobol_tables[base + 512u + ((index >> 16u) & 0xffu)]
        ^ sobol_tables[base + 768u + (index >> 24u)];
}

fn laine_karras_permutation(v: u32, seed: u32) -> u32 {
    var x = v + seed;
    x ^= x * 0x6c50b47cu;
    x ^= x * 0xb82f1e52u;
    x ^= x * 0xc7afe638u;
    x ^= x * 0x8d22f6e6u;
    return x;
}

fn nested_uniform_scramble(x: u32, seed: u32) -> u32 {
    return reverseBits(laine_karras_permutation(reverseBits(x), seed));
}

// sobol::Group: a dimension group's seed and the index shuffled by it.
struct SobolGroup {
    seed: u32,
    shuffled: u32,
};

fn sobol_shuffle(index: u32, group: u32, seed: u32) -> SobolGroup {
    let group_seed = hash_combine(seed, group * 0x51633e2du);
    return SobolGroup(group_seed, nested_uniform_scramble(index, hash_u32(group_seed)));
}

fn sobol_sample_u32_in(group: SobolGroup, base_dim: u32) -> u32 {
    let x = sobol_u32(group.shuffled, base_dim);
    return nested_uniform_scramble(x, hash_combine(group.seed, base_dim ^ 0xa511e9b3u));
}

fn sobol_sample_u32(index: u32, dimension: u32, seed: u32) -> u32 {
    let group = sobol_shuffle(index, dimension / NUM_BASE_DIMENSIONS, seed);
    return sobol_sample_u32_in(group, dimension % NUM_BASE_DIMENSIONS);
}
