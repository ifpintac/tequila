// Blender's shader-node textures and converters, for the graphics card.
//
// A GLSL translation of `tequila-texture/src/cycles` - itself a translation
// of Cycles' SVM kernels, Copyright 2011-2022 Blender Foundation,
// SPDX-License-Identifier: Apache-2.0 - so that the viewport's patterns are
// the renderer's: the same hashes, the same noise, the same cells. Kept
// line for line with the Rust where a difference would show: integer hashes
// in unsigned arithmetic, truncated modulo where Rust's `%` truncates.

// ---- hashes: Bob Jenkins' lookup3, as Cycles uses it ------------------------

uint fs_rot(uint x, uint k) { return (x << k) | (x >> (32u - k)); }

void fs_final(inout uint a, inout uint b, inout uint c) {
    c ^= b; c -= fs_rot(b, 14u);
    a ^= c; a -= fs_rot(c, 11u);
    b ^= a; b -= fs_rot(a, 25u);
    c ^= b; c -= fs_rot(b, 16u);
    a ^= c; a -= fs_rot(c, 4u);
    b ^= a; b -= fs_rot(a, 14u);
    c ^= b; c -= fs_rot(b, 24u);
}

void fs_mix_hash(inout uint a, inout uint b, inout uint c) {
    a -= c; a ^= fs_rot(c, 4u); c += b;
    b -= a; b ^= fs_rot(a, 6u); a += c;
    c -= b; c ^= fs_rot(b, 8u); b += a;
    a -= c; a ^= fs_rot(c, 16u); c += b;
    b -= a; b ^= fs_rot(a, 19u); a += c;
    c -= b; c ^= fs_rot(b, 4u); b += a;
}

const uint FS_SEED = 0xdeadbeefu;

uint fs_hash1(uint kx) {
    uint a = FS_SEED + 17u, b = a, c = a;
    a += kx;
    fs_final(a, b, c);
    return c;
}
uint fs_hash2(uint kx, uint ky) {
    uint a = FS_SEED + 21u, b = a, c = a;
    b += ky; a += kx;
    fs_final(a, b, c);
    return c;
}
uint fs_hash3(uint kx, uint ky, uint kz) {
    uint a = FS_SEED + 25u, b = a, c = a;
    c += kz; b += ky; a += kx;
    fs_final(a, b, c);
    return c;
}
uint fs_hash4(uint kx, uint ky, uint kz, uint kw) {
    uint a = FS_SEED + 29u, b = a, c = a;
    a += kx; b += ky; c += kz;
    fs_mix_hash(a, b, c);
    a += kw;
    fs_final(a, b, c);
    return c;
}

float fs_u2f(uint n) { return float(n) * (1.0 / 4294967295.0); }
float fs_hashf1(float k) { return fs_u2f(fs_hash1(floatBitsToUint(k))); }
float fs_hashf2(vec2 k) { return fs_u2f(fs_hash2(floatBitsToUint(k.x), floatBitsToUint(k.y))); }
float fs_hashf3(vec3 k) { return fs_u2f(fs_hash3(floatBitsToUint(k.x), floatBitsToUint(k.y), floatBitsToUint(k.z))); }
float fs_hashf4(vec4 k) { return fs_u2f(fs_hash4(floatBitsToUint(k.x), floatBitsToUint(k.y), floatBitsToUint(k.z), floatBitsToUint(k.w))); }

vec3 fs_hashf1_3(float k) { return vec3(fs_hashf1(k), fs_hashf2(vec2(k, 1.0)), fs_hashf2(vec2(k, 2.0))); }
vec3 fs_hashf2_3(vec2 k) { return vec3(fs_hashf2(k), fs_hashf3(vec3(k, 1.0)), fs_hashf3(vec3(k, 2.0))); }
vec3 fs_hashf3_3(vec3 k) { return vec3(fs_hashf3(k), fs_hashf4(vec4(k, 1.0)), fs_hashf4(vec4(k, 2.0))); }
vec3 fs_hashf4_3(vec4 k) { return vec3(fs_hashf4(k), fs_hashf4(vec4(k.z, k.x, k.w, k.y)), fs_hashf4(vec4(k.w, k.z, k.y, k.x))); }

// PCG, for Voronoi's cells: multiplications in unsigned arithmetic, which
// wraps as Rust's wrapping_mul does; the shifts on signed values, which
// extend the sign as Rust's `>>` on i32 does.
const float FS_PCG_SCALE = 1.0 / 2147483647.0;

vec2 fs_pcg2(ivec2 v) {
    uvec2 u = uvec2(v) * 1664525u + 1013904223u;
    u.x += u.y * 1664525u;
    u.y += u.x * 1664525u;
    ivec2 s = ivec2(u);
    s ^= s >> 16;
    u = uvec2(s);
    u.x += u.y * 1664525u;
    u.y += u.x * 1664525u;
    return vec2(u & 0x7FFFFFFFu) * FS_PCG_SCALE;
}
vec3 fs_pcg3(ivec3 v) {
    uvec3 u = uvec3(v) * 1664525u + 1013904223u;
    u.x += u.y * u.z;
    u.y += u.z * u.x;
    u.z += u.x * u.y;
    ivec3 s = ivec3(u);
    s ^= s >> 16;
    u = uvec3(s);
    u.x += u.y * u.z;
    u.y += u.z * u.x;
    u.z += u.x * u.y;
    return vec3(u & 0x7FFFFFFFu) * FS_PCG_SCALE;
}
vec4 fs_pcg4(ivec4 v) {
    uvec4 u = uvec4(v) * 1664525u + 1013904223u;
    u.x += u.y * u.w;
    u.y += u.z * u.x;
    u.z += u.x * u.y;
    u.w += u.y * u.z;
    ivec4 s = ivec4(u);
    s ^= s >> 16;
    u = uvec4(s);
    u.x += u.y * u.w;
    u.y += u.z * u.x;
    u.z += u.x * u.y;
    u.w += u.y * u.z;
    return vec4(u & 0x7FFFFFFFu) * FS_PCG_SCALE;
}

// ---- Perlin noise -------------------------------------------------------------

float fs_fade(float t) { return t * t * t * (t * (t * 6.0 - 15.0) + 10.0); }
float fs_negate_if(float v, int c) { return c != 0 ? -v : v; }

float fs_grad1(uint hash, float x) {
    int h = int(hash & 15u);
    float g = 1.0 + float(h & 7);
    return fs_negate_if(g, h & 8) * x;
}
float fs_grad2(uint hash, float x, float y) {
    int h = int(hash & 7u);
    float u = h < 4 ? x : y;
    float v = 2.0 * (h < 4 ? y : x);
    return fs_negate_if(u, h & 1) + fs_negate_if(v, h & 2);
}
float fs_grad3(uint hash, float x, float y, float z) {
    int h = int(hash & 15u);
    float u = h < 8 ? x : y;
    float vt = (h == 12 || h == 14) ? x : z;
    float v = h < 4 ? y : vt;
    return fs_negate_if(u, h & 1) + fs_negate_if(v, h & 2);
}
float fs_grad4(uint hash, float x, float y, float z, float w) {
    int h = int(hash & 31u);
    float u = h < 24 ? x : y;
    float v = h < 16 ? y : z;
    float s = h < 8 ? z : w;
    return fs_negate_if(u, h & 1) + fs_negate_if(v, h & 2) + fs_negate_if(s, h & 4);
}

float fs_perlin1(float x) {
    float fl = floor(x);
    float f = x - fl;
    int i = int(fl);
    float u = fs_fade(f);
    float a = fs_grad1(fs_hash1(uint(i)), f);
    float b = fs_grad1(fs_hash1(uint(i + 1)), f - 1.0);
    return a + u * (b - a);
}

float fs_perlin2(vec2 p) {
    vec2 fl = floor(p);
    vec2 f = p - fl;
    ivec2 i = ivec2(fl);
    float u = fs_fade(f.x), v = fs_fade(f.y);
    float v0 = fs_grad2(fs_hash2(uint(i.x), uint(i.y)), f.x, f.y);
    float v1 = fs_grad2(fs_hash2(uint(i.x + 1), uint(i.y)), f.x - 1.0, f.y);
    float v2 = fs_grad2(fs_hash2(uint(i.x), uint(i.y + 1)), f.x, f.y - 1.0);
    float v3 = fs_grad2(fs_hash2(uint(i.x + 1), uint(i.y + 1)), f.x - 1.0, f.y - 1.0);
    float x1 = 1.0 - u;
    return (1.0 - v) * (v0 * x1 + v1 * u) + v * (v2 * x1 + v3 * u);
}

float fs_g3(ivec3 i, vec3 f, int dx, int dy, int dz) {
    return fs_grad3(fs_hash3(uint(i.x + dx), uint(i.y + dy), uint(i.z + dz)), f.x - float(dx), f.y - float(dy), f.z - float(dz));
}

float fs_tri_mix(float v0, float v1, float v2, float v3, float v4, float v5, float v6, float v7, float x, float y, float z) {
    float x1 = 1.0 - x, y1 = 1.0 - y, z1 = 1.0 - z;
    return z1 * (y1 * (v0 * x1 + v1 * x) + y * (v2 * x1 + v3 * x))
         + z * (y1 * (v4 * x1 + v5 * x) + y * (v6 * x1 + v7 * x));
}

float fs_perlin3(vec3 p) {
    vec3 fl = floor(p);
    vec3 f = p - fl;
    ivec3 i = ivec3(fl);
    float u = fs_fade(f.x), v = fs_fade(f.y), w = fs_fade(f.z);
    return fs_tri_mix(
        fs_g3(i, f, 0, 0, 0), fs_g3(i, f, 1, 0, 0), fs_g3(i, f, 0, 1, 0), fs_g3(i, f, 1, 1, 0),
        fs_g3(i, f, 0, 0, 1), fs_g3(i, f, 1, 0, 1), fs_g3(i, f, 0, 1, 1), fs_g3(i, f, 1, 1, 1),
        u, v, w);
}

float fs_g4(ivec4 i, vec4 f, int dx, int dy, int dz, int dw) {
    return fs_grad4(fs_hash4(uint(i.x + dx), uint(i.y + dy), uint(i.z + dz), uint(i.w + dw)),
        f.x - float(dx), f.y - float(dy), f.z - float(dz), f.w - float(dw));
}

float fs_perlin4(vec4 p) {
    vec4 fl = floor(p);
    vec4 f = p - fl;
    ivec4 i = ivec4(fl);
    float u = fs_fade(f.x), v = fs_fade(f.y), t = fs_fade(f.z), s = fs_fade(f.w);
    float near = fs_tri_mix(
        fs_g4(i, f, 0, 0, 0, 0), fs_g4(i, f, 1, 0, 0, 0), fs_g4(i, f, 0, 1, 0, 0), fs_g4(i, f, 1, 1, 0, 0),
        fs_g4(i, f, 0, 0, 1, 0), fs_g4(i, f, 1, 0, 1, 0), fs_g4(i, f, 0, 1, 1, 0), fs_g4(i, f, 1, 1, 1, 0),
        u, v, t);
    float far = fs_tri_mix(
        fs_g4(i, f, 0, 0, 0, 1), fs_g4(i, f, 1, 0, 0, 1), fs_g4(i, f, 0, 1, 0, 1), fs_g4(i, f, 1, 1, 0, 1),
        fs_g4(i, f, 0, 0, 1, 1), fs_g4(i, f, 1, 0, 1, 1), fs_g4(i, f, 0, 1, 1, 1), fs_g4(i, f, 1, 1, 1, 1),
        u, v, t);
    return near + s * (far - near);
}

// Rust's `%` truncates; GLSL's mod floors.
float fs_wrap(float p) {
    float correction = abs(p) >= 1000000.0 ? 0.5 : 0.0;
    return (p - 100000.0 * trunc(p / 100000.0)) + correction;
}
vec2 fs_wrap(vec2 p) { return vec2(fs_wrap(p.x), fs_wrap(p.y)); }
vec3 fs_wrap(vec3 p) { return vec3(fs_wrap(p.x), fs_wrap(p.y), fs_wrap(p.z)); }
vec4 fs_wrap(vec4 p) { return vec4(fs_wrap(p.x), fs_wrap(p.y), fs_wrap(p.z), fs_wrap(p.w)); }

float fs_snoise(float p) { return 0.2500 * fs_perlin1(fs_wrap(p)); }
float fs_snoise(vec2 p) { return 0.6616 * fs_perlin2(fs_wrap(p)); }
float fs_snoise(vec3 p) { return 0.9820 * fs_perlin3(fs_wrap(p)); }
float fs_snoise(vec4 p) { return 0.8344 * fs_perlin4(fs_wrap(p)); }

float fs_offset_n(float seed, float i) { return 100.0 + fs_hashf2(vec2(seed, i)) * 100.0; }
float fs_seed_offset1(float seed) { return 100.0 + fs_hashf1(seed) * 100.0; }
vec2 fs_seed_offset2(float seed) { return vec2(fs_offset_n(seed, 0.0), fs_offset_n(seed, 1.0)); }
vec3 fs_seed_offset3(float seed) { return vec3(fs_offset_n(seed, 0.0), fs_offset_n(seed, 1.0), fs_offset_n(seed, 2.0)); }
vec4 fs_seed_offset4(float seed) { return vec4(fs_offset_n(seed, 0.0), fs_offset_n(seed, 1.0), fs_offset_n(seed, 2.0), fs_offset_n(seed, 3.0)); }

float fs_distort(float p, float amount) { return p + fs_snoise(p + fs_seed_offset1(0.0)) * amount; }
vec2 fs_distort(vec2 p, float amount) {
    return p + vec2(fs_snoise(p + fs_seed_offset2(0.0)), fs_snoise(p + fs_seed_offset2(1.0))) * amount;
}
vec3 fs_distort(vec3 p, float amount) {
    return p + vec3(fs_snoise(p + fs_seed_offset3(0.0)), fs_snoise(p + fs_seed_offset3(1.0)), fs_snoise(p + fs_seed_offset3(2.0))) * amount;
}
vec4 fs_distort(vec4 p, float amount) {
    return p + vec4(fs_snoise(p + fs_seed_offset4(0.0)), fs_snoise(p + fs_seed_offset4(1.0)),
        fs_snoise(p + fs_seed_offset4(2.0)), fs_snoise(p + fs_seed_offset4(3.0))) * amount;
}

// How far the point moves across a pixel, in its own units: how fine a
// pattern can be before the pixel averages it away.
float fs_footprint(float p) { return fwidth(p); }
float fs_footprint(vec2 p) { vec2 w = fwidth(p); return max(w.x, w.y); }
float fs_footprint(vec3 p) { vec3 w = fwidth(p); return max(w.x, max(w.y, w.z)); }
float fs_footprint(vec4 p) { vec4 w = fwidth(p); return max(max(w.x, w.y), max(w.z, w.w)); }

// The fractal sums, once per dimension: GLSL has overloading but not
// generics, so the Rust generates these from one template (`FRACTALS`).
@FRACTALS@

// ---- Voronoi, in three dimensions -----------------------------------------------

struct FsVoronoi {
    float distance;
    vec3 color;
    vec3 position;
};

float fs_vdist(vec3 a, vec3 b, int metric, float exponent) {
    vec3 d = abs(a - b);
    if (metric == 0) return length(a - b);
    if (metric == 1) return d.x + d.y + d.z;
    if (metric == 2) return max(d.x, max(d.y, d.z));
    return pow(pow(d.x, exponent) + pow(d.y, exponent) + pow(d.z, exponent), 1.0 / exponent);
}

float fs_vbound(vec3 a, vec3 b, int metric, float exponent) {
    vec3 d = abs(a - b);
    if (metric == 0) return dot(a - b, a - b);
    if (metric == 1) return d.x + d.y + d.z;
    if (metric == 2) return max(d.x, max(d.y, d.z));
    return pow(d.x, exponent) + pow(d.y, exponent) + pow(d.z, exponent);
}

FsVoronoi fs_vor_f1(vec3 coord, int metric, float exponent, float randomness) {
    vec3 fl = floor(coord);
    vec3 local = coord - fl;
    ivec3 cell = ivec3(fl);
    float best = 3.4e38;
    ivec3 target_offset = ivec3(0);
    vec3 target = vec3(0.0);
    for (int k = -1; k <= 1; k++) for (int j = -1; j <= 1; j++) for (int i = -1; i <= 1; i++) {
        ivec3 offset = ivec3(i, j, k);
        vec3 point = vec3(offset) + fs_pcg3(cell + offset) * randomness;
        float d = fs_vbound(point, local, metric, exponent);
        if (d < best) { target_offset = offset; best = d; target = point; }
    }
    FsVoronoi o;
    o.distance = fs_vdist(target, local, metric, exponent);
    o.color = fs_pcg3(cell + target_offset);
    o.position = target + fl;
    return o;
}

FsVoronoi fs_vor_smooth_f1(vec3 coord, int metric, float exponent, float randomness, float smoothness) {
    vec3 fl = floor(coord);
    vec3 local = coord - fl;
    ivec3 cell = ivec3(fl);
    float sd = 0.0;
    vec3 sc = vec3(0.0), sp = vec3(0.0);
    float h = -1.0;
    for (int k = -2; k <= 2; k++) for (int j = -2; j <= 2; j++) for (int i = -2; i <= 2; i++) {
        ivec3 offset = ivec3(i, j, k);
        vec3 point = vec3(offset) + fs_pcg3(cell + offset) * randomness;
        float d = fs_vdist(point, local, metric, exponent);
        h = h == -1.0 ? 1.0 : smoothstep(0.0, 1.0, 0.5 + 0.5 * (sd - d) / smoothness);
        float correction = smoothness * h * (1.0 - h);
        sd = mix(sd, d, h) - correction;
        correction /= 1.0 + 3.0 * smoothness;
        sc = mix(sc, fs_pcg3(cell + offset), h) - correction;
        sp = mix(sp, point, h) - correction;
    }
    FsVoronoi o;
    o.distance = sd;
    o.color = sc;
    o.position = fl + sp;
    return o;
}

FsVoronoi fs_vor_f2(vec3 coord, int metric, float exponent, float randomness) {
    vec3 fl = floor(coord);
    vec3 local = coord - fl;
    ivec3 cell = ivec3(fl);
    float d1 = 3.4e38, d2 = 3.4e38;
    ivec3 off1 = ivec3(0), off2 = ivec3(0);
    vec3 pos1 = vec3(0.0), pos2 = vec3(0.0);
    for (int k = -1; k <= 1; k++) for (int j = -1; j <= 1; j++) for (int i = -1; i <= 1; i++) {
        ivec3 offset = ivec3(i, j, k);
        vec3 point = vec3(offset) + fs_pcg3(cell + offset) * randomness;
        float d = fs_vdist(point, local, metric, exponent);
        if (d < d1) {
            d2 = d1; d1 = d; off2 = off1; off1 = offset; pos2 = pos1; pos1 = point;
        } else if (d < d2) {
            d2 = d; off2 = offset; pos2 = point;
        }
    }
    FsVoronoi o;
    o.distance = d2;
    o.color = fs_pcg3(cell + off2);
    o.position = pos2 + fl;
    return o;
}

float fs_vor_edge(vec3 coord, float randomness) {
    vec3 fl = floor(coord);
    vec3 local = coord - fl;
    ivec3 cell = ivec3(fl);
    vec3 to_closest = vec3(0.0);
    float best = 3.4e38;
    for (int k = -1; k <= 1; k++) for (int j = -1; j <= 1; j++) for (int i = -1; i <= 1; i++) {
        ivec3 offset = ivec3(i, j, k);
        vec3 v = vec3(offset) + fs_pcg3(cell + offset) * randomness - local;
        float d = dot(v, v);
        if (d < best) { best = d; to_closest = v; }
    }
    best = 3.4e38;
    for (int k = -1; k <= 1; k++) for (int j = -1; j <= 1; j++) for (int i = -1; i <= 1; i++) {
        ivec3 offset = ivec3(i, j, k);
        vec3 v = vec3(offset) + fs_pcg3(cell + offset) * randomness - local;
        vec3 perpendicular = v - to_closest;
        float len2 = dot(perpendicular, perpendicular);
        if (len2 > 0.0001) {
            best = min(best, dot((to_closest + v) / 2.0, perpendicular / sqrt(len2)));
        }
    }
    return best;
}

float fs_vor_radius(vec3 coord, float randomness) {
    vec3 fl = floor(coord);
    vec3 local = coord - fl;
    ivec3 cell = ivec3(fl);
    vec3 closest = vec3(0.0);
    ivec3 closest_offset = ivec3(0);
    float best = 3.4e38;
    for (int k = -1; k <= 1; k++) for (int j = -1; j <= 1; j++) for (int i = -1; i <= 1; i++) {
        ivec3 offset = ivec3(i, j, k);
        vec3 point = vec3(offset) + fs_pcg3(cell + offset) * randomness;
        float d = dot(point - local, point - local);
        if (d < best) { best = d; closest = point; closest_offset = offset; }
    }
    best = 3.4e38;
    vec3 nearest = vec3(0.0);
    for (int k = -1; k <= 1; k++) for (int j = -1; j <= 1; j++) for (int i = -1; i <= 1; i++) {
        if (i == 0 && j == 0 && k == 0) continue;
        ivec3 offset = ivec3(i, j, k) + closest_offset;
        vec3 point = vec3(offset) + fs_pcg3(cell + offset) * randomness;
        float d = dot(closest - point, closest - point);
        if (d < best) { best = d; nearest = point; }
    }
    return length(nearest - closest) / 2.0;
}

// feature: 0 F1, 1 F2, 2 smooth F1.
FsVoronoi fs_vor_pick(int feature, vec3 coord, int metric, float exponent, float randomness, float smoothness) {
    if (feature == 1) return fs_vor_f2(coord, metric, exponent, randomness);
    if (feature == 2 && smoothness != 0.0) return fs_vor_smooth_f1(coord, metric, exponent, randomness, smoothness);
    return fs_vor_f1(coord, metric, exponent, randomness);
}

FsVoronoi fs_voronoi3(vec3 coord, float scale, float detail, float roughness, float lacunarity,
                      float smoothness, float exponent, float randomness, int feature, int metric, bool normalize) {
    detail = clamp(detail, 0.0, 15.0);
    roughness = clamp(roughness, 0.0, 1.0);
    smoothness = clamp(smoothness / 2.0, 0.0, 0.5);
    randomness = clamp(randomness, 0.0, 1.0);
    float reach = 0.5 + 0.5 * randomness;
    float max_distance = fs_vdist(vec3(0.0), vec3(reach), metric, exponent) * (feature == 1 ? 2.0 : 1.0);
    float amplitude = 1.0, max_amplitude = 0.0, octave_scale = 1.0;
    FsVoronoi o;
    o.distance = 0.0; o.color = vec3(0.0); o.position = vec3(0.0);
    bool zero_input = detail == 0.0 || roughness == 0.0;
    for (int i = 0; float(i) <= ceil(detail); i++) {
        FsVoronoi octave = fs_vor_pick(feature, coord * octave_scale, metric, exponent, randomness, smoothness);
        if (zero_input) { max_amplitude = 1.0; o = octave; break; }
        if (float(i) <= detail) {
            max_amplitude += amplitude;
            o.distance += octave.distance * amplitude;
            o.color += octave.color * amplitude;
            o.position = mix(o.position, octave.position / octave_scale, amplitude);
            octave_scale *= lacunarity;
            amplitude *= roughness;
        } else {
            float remainder = detail - floor(detail);
            if (remainder != 0.0) {
                max_amplitude = mix(max_amplitude, max_amplitude + amplitude, remainder);
                o.distance = mix(o.distance, o.distance + octave.distance * amplitude, remainder);
                o.color = mix(o.color, o.color + octave.color * amplitude, remainder);
                o.position = mix(o.position, mix(o.position, octave.position / octave_scale, amplitude), remainder);
            }
        }
    }
    if (normalize) {
        o.distance /= max_amplitude * max_distance;
        o.color /= max_amplitude;
    }
    o.position = scale != 0.0 ? o.position / scale : vec3(0.0);
    return o;
}

float fs_voronoi3_edge(vec3 coord, float detail, float roughness, float lacunarity, float randomness, bool normalize) {
    detail = clamp(detail, 0.0, 15.0);
    roughness = clamp(roughness, 0.0, 1.0);
    randomness = clamp(randomness, 0.0, 1.0);
    float max_distance = 0.5 + 0.5 * randomness;
    float amplitude = 1.0, max_amplitude = max_distance, octave_scale = 1.0, distance = 8.0;
    bool zero_input = detail == 0.0 || roughness == 0.0;
    for (int i = 0; float(i) <= ceil(detail); i++) {
        float octave = fs_vor_edge(coord * octave_scale, randomness);
        if (zero_input) { distance = octave; break; }
        if (float(i) <= detail) {
            max_amplitude = mix(max_amplitude, max_distance / octave_scale, amplitude);
            distance = mix(distance, min(distance, octave / octave_scale), amplitude);
            octave_scale *= lacunarity;
            amplitude *= roughness;
        } else {
            float remainder = detail - floor(detail);
            if (remainder != 0.0) {
                float lerp_amplitude = mix(max_amplitude, max_distance / octave_scale, amplitude);
                max_amplitude = mix(max_amplitude, lerp_amplitude, remainder);
                float lerp_distance = mix(distance, min(distance, octave / octave_scale), amplitude);
                distance = mix(distance, min(distance, lerp_distance), remainder);
            }
        }
    }
    if (normalize) distance /= max_amplitude;
    return distance;
}

// ---- the other textures ------------------------------------------------------------

float fs_white1(float w) { return fs_hashf1(w); }

// kind: 0 bands, 1 rings; direction: 0 x, 1 y, 2 z, 3 diagonal/spherical;
// profile: 0 sine, 1 saw, 2 triangle.
float fs_wave(vec3 p, int kind, int direction, int profile, float distortion, float detail,
              float detail_scale, float detail_roughness, float phase) {
    p = (p + 0.000001) * 0.999999;
    float n;
    if (kind == 0) {
        if (direction == 0) n = p.x * 20.0;
        else if (direction == 1) n = p.y * 20.0;
        else if (direction == 2) n = p.z * 20.0;
        else n = (p.x + p.y + p.z) * 10.0;
    } else {
        vec3 rp = p;
        if (direction == 0) rp.x = 0.0;
        else if (direction == 1) rp.y = 0.0;
        else if (direction == 2) rp.z = 0.0;
        n = length(rp) * 20.0;
    }
    n += phase;
    if (distortion != 0.0) {
        n += distortion * (fs_fbm(p * detail_scale, detail, detail_roughness, 2.0, true, fs_footprint(p * detail_scale)) * 2.0 - 1.0);
    }
    if (profile == 0) return 0.5 + 0.5 * sin(n - 1.5707963);
    if (profile == 1) { n /= 6.2831853; return n - floor(n); }
    n /= 6.2831853;
    return abs(n - floor(n + 0.5)) * 2.0;
}

float fs_trunc_mod(float a, float b) { return a - b * trunc(a / b); }

vec3 fs_magic(vec3 p, float scale, int depth, float distortion) {
    float px = fs_trunc_mod(p.x, 6.2831853);
    float py = fs_trunc_mod(p.y, 6.2831853);
    float pz = fs_trunc_mod(p.z, 6.2831853);
    float x = sin((px + py + pz) * 5.0 * scale);
    float y = cos((-px + py - pz) * 5.0 * scale);
    float z = -cos((-px - py + pz) * 5.0 * scale);
    if (depth > 0) {
        x *= distortion; y *= distortion; z *= distortion;
        y = -cos(x - y + z); y *= distortion;
        if (depth > 1) { x = cos(x - y - z); x *= distortion;
        if (depth > 2) { z = sin(-x - y - z); z *= distortion;
        if (depth > 3) { x = -cos(-x + y - z); x *= distortion;
        if (depth > 4) { y = -sin(-x + y + z); y *= distortion;
        if (depth > 5) { y = -cos(-x + y + z); y *= distortion;
        if (depth > 6) { x = cos(x + y + z); x *= distortion;
        if (depth > 7) { z = sin(x + y - z); z *= distortion;
        if (depth > 8) { x = -cos(-x - y + z); x *= distortion;
        if (depth > 9) { y = -sin(x - y + z); y *= distortion; }}}}}}}}}
    }
    if (distortion != 0.0) {
        distortion *= 2.0;
        x /= distortion; y /= distortion; z /= distortion;
    }
    return vec3(0.5 - x, 0.5 - y, 0.5 - z);
}

float fs_brick_noise(uint n) {
    n = (n + 1013u) & 0x7fffffffu;
    n ^= n >> 13;
    uint nn = (n * (n * n * 60493u + 19990303u) + 1376312589u) & 0x7fffffffu;
    return 0.5 * (float(nn) / 1073741824.0);
}

// Rust's `%` on integers truncates; GLSL leaves negative operands undefined.
int fs_imod(int a, int b) { return a - b * int(trunc(float(a) / float(b))); }

// The brick's tint and mortar.
vec2 fs_brick(vec3 p, float mortar_size, float mortar_smooth, float bias, float brick_width, float row_height,
              float offset_amount, int offset_frequency, float squash_amount, int squash_frequency) {
    int rownum = int(floor(p.y / row_height));
    float offset = 0.0;
    if (offset_frequency != 0 && squash_frequency != 0) {
        brick_width *= fs_imod(rownum, squash_frequency) != 0 ? 1.0 : squash_amount;
        offset = fs_imod(rownum, offset_frequency) != 0 ? 0.0 : brick_width * offset_amount;
    }
    int bricknum = int(floor((p.x + offset) / brick_width));
    float x = (p.x + offset) - brick_width * float(bricknum);
    float y = p.y - row_height * float(rownum);
    uint key = uint((rownum << 16) + (bricknum & 0xFFFF));
    float tint = clamp(fs_brick_noise(key) + bias, 0.0, 1.0);
    float min_dist = min(min(x, y), min(brick_width - x, row_height - y));
    float mortar;
    if (min_dist >= mortar_size) mortar = 0.0;
    else if (mortar_smooth == 0.0) mortar = 1.0;
    else {
        min_dist = 1.0 - min_dist / mortar_size;
        float f = min_dist / mortar_smooth;
        mortar = f <= 0.0 ? 0.0 : (f >= 1.0 ? 1.0 : 3.0 * f * f - 2.0 * f * f * f);
    }
    return vec2(tint, mortar);
}

float fs_checker(vec3 p) {
    p = (p + 0.000001) * 0.999999;
    int xi = abs(int(floor(p.x)));
    int yi = abs(int(floor(p.y)));
    int zi = abs(int(floor(p.z)));
    return ((xi % 2 == yi % 2) == (zi % 2 != 0)) ? 1.0 : 0.0;
}

// kind: 0 linear, 1 quadratic, 2 easing, 3 diagonal, 4 spherical,
// 5 quadratic sphere, 6 radial.
float fs_gradient_texture(vec3 p, int kind) {
    float f;
    if (kind == 0) f = p.x;
    else if (kind == 1) { float r = max(p.x, 0.0); f = r * r; }
    else if (kind == 2) { float r = clamp(p.x, 0.0, 1.0); float t = r * r; f = 3.0 * t - 2.0 * t * r; }
    else if (kind == 3) f = (p.x + p.y) * 0.5;
    else if (kind == 6) f = atan(p.y, p.x) / 6.2831853 + 0.5;
    else { float r = max(0.999999 - length(p), 0.0); f = kind == 5 ? r * r : r; }
    return clamp(f, 0.0, 1.0);
}

// ---- converters ------------------------------------------------------------------------

float fs_safe_divide(float a, float b) { return b != 0.0 ? a / b : 0.0; }
float fs_compatible_pow(float x, float y) { return y == 0.0 ? 1.0 : (x == 0.0 ? 0.0 : pow(x, y)); }
float fs_safe_pow(float a, float b) { return (a < 0.0 && b != float(int(b))) ? 0.0 : (a < 0.0 ? (mod(abs(b), 2.0) == 1.0 ? -pow(-a, b) : pow(-a, b)) : fs_compatible_pow(a, b)); }
float fs_safe_log(float a, float b) { return (a <= 0.0 || b <= 0.0) ? 0.0 : fs_safe_divide(log(a), log(b)); }
float fs_sign(float f) { return f == 0.0 ? 0.0 : (f < 0.0 ? -1.0 : 1.0); }
float fs_wrapf(float value, float maxv, float minv) {
    float range = maxv - minv;
    return range != 0.0 ? value - range * floor((value - minv) / range) : minv;
}
float fs_pingpong(float a, float b) { return b != 0.0 ? abs(fract((a - b) / (b * 2.0)) * b * 2.0 - b) : 0.0; }
float fs_smoothmin(float a, float b, float k) {
    if (k != 0.0) { float h = max(k - abs(a - b), 0.0) / k; return min(a, b) - h * h * h * k * (1.0 / 6.0); }
    return min(a, b);
}
float fs_floored_mod(float a, float b) { return b != 0.0 ? a - floor(a / b) * b : 0.0; }
float fs_smoothstep_c(float e0, float e1, float x) {
    if (x < e0) return 0.0;
    if (x >= e1) return 1.0;
    float t = (x - e0) / (e1 - e0);
    return (3.0 - 2.0 * t) * (t * t);
}
float fs_smootherstep(float e0, float e1, float x) {
    x = clamp(fs_safe_divide(x - e0, e1 - e0), 0.0, 1.0);
    return x * x * x * (x * (x * 6.0 - 15.0) + 10.0);
}

// kind: 0 linear, 1 stepped, 2 smoothstep, 3 smootherstep.
float fs_map_range(int kind, float value, float from_min, float from_max, float to_min, float to_max, float steps, bool clamped) {
    if (from_max == from_min) return 0.0;
    float factor;
    if (kind == 0) factor = (value - from_min) / (from_max - from_min);
    else if (kind == 1) {
        float f = (value - from_min) / (from_max - from_min);
        factor = steps > 0.0 ? floor(f * (steps + 1.0)) / steps : 0.0;
    } else if (kind == 2) {
        factor = from_min > from_max ? 1.0 - fs_smoothstep_c(from_max, from_min, value) : fs_smoothstep_c(from_min, from_max, value);
    } else {
        factor = from_min > from_max ? 1.0 - fs_smootherstep(from_max, from_min, value) : fs_smootherstep(from_min, from_max, value);
    }
    float result = to_min + factor * (to_max - to_min);
    if (clamped && kind <= 1) result = clamp(result, min(to_min, to_max), max(to_min, to_max));
    return result;
}

float fs_clamp_node(float value, float lo, float hi, bool range) {
    return (range && lo > hi) ? clamp(value, hi, lo) : min(max(value, lo), hi);
}

mat3 fs_euler(vec3 e) {
    float cx = cos(e.x), cy = cos(e.y), cz = cos(e.z);
    float sx = sin(e.x), sy = sin(e.y), sz = sin(e.z);
    // Rows, as the Rust has them; GLSL's mat3 takes columns.
    return transpose(mat3(
        cy * cz, sy * sx * cz - cx * sz, sy * cx * cz + sx * sz,
        cy * sz, sy * sx * sz + cx * cz, sy * cx * sz - sx * cz,
        -sy, cy * sx, cy * cx));
}

vec3 fs_safe_divide3(vec3 a, vec3 b) { return vec3(fs_safe_divide(a.x, b.x), fs_safe_divide(a.y, b.y), fs_safe_divide(a.z, b.z)); }

// kind: 0 point, 1 texture, 2 vector, 3 normal.
vec3 fs_mapping(int kind, vec3 v, vec3 location, vec3 rotation, vec3 scale) {
    mat3 r = fs_euler(rotation);
    if (kind == 0) return r * (v * scale) + location;
    if (kind == 1) return fs_safe_divide3(transpose(r) * (v - location), scale);
    if (kind == 2) return r * (v * scale);
    vec3 n = r * fs_safe_divide3(v, scale);
    float t = length(n);
    return t != 0.0 ? n / t : n;
}

vec3 fs_rgb_to_hsv(vec3 rgb) {
    float cmax = max(rgb.r, max(rgb.g, rgb.b));
    float cmin = min(rgb.r, min(rgb.g, rgb.b));
    float cdelta = cmax - cmin;
    float s = cmax != 0.0 ? cdelta / cmax : 0.0;
    float h = 0.0;
    if (s != 0.0) {
        vec3 c = (vec3(cmax) - rgb) / cdelta;
        if (rgb.r == cmax) h = c.b - c.g;
        else if (rgb.g == cmax) h = 2.0 + c.r - c.b;
        else h = 4.0 + c.g - c.r;
        h /= 6.0;
        if (h < 0.0) h += 1.0;
    }
    return vec3(h, s, cmax);
}

vec3 fs_hsv_to_rgb(vec3 hsv) {
    float h = hsv.x, s = hsv.y, v = hsv.z;
    if (s == 0.0) return vec3(v);
    if (h == 1.0) h = 0.0;
    h *= 6.0;
    float i = floor(h);
    float f = h - i;
    float p = v * (1.0 - s), q = v * (1.0 - s * f), t = v * (1.0 - s * (1.0 - f));
    if (i == 0.0) return vec3(v, t, p);
    if (i == 1.0) return vec3(q, v, p);
    if (i == 2.0) return vec3(p, v, t);
    if (i == 3.0) return vec3(p, q, v);
    if (i == 4.0) return vec3(t, p, v);
    return vec3(v, p, q);
}

vec3 fs_rgb_to_hsl(vec3 rgb) {
    float cmax = max(rgb.r, max(rgb.g, rgb.b));
    float cmin = min(rgb.r, min(rgb.g, rgb.b));
    float l = min(1.0, (cmax + cmin) / 2.0);
    float h = 0.0, s = 0.0;
    if (cmax != cmin) {
        float cdelta = cmax - cmin;
        s = l > 0.5 ? cdelta / (2.0 - cmax - cmin) : cdelta / (cmax + cmin);
        if (cmax == rgb.r) h = (rgb.g - rgb.b) / cdelta + (rgb.g < rgb.b ? 6.0 : 0.0);
        else if (cmax == rgb.g) h = (rgb.b - rgb.r) / cdelta + 2.0;
        else h = (rgb.r - rgb.g) / cdelta + 4.0;
    }
    return vec3(h / 6.0, s, l);
}

vec3 fs_hsl_to_rgb(vec3 hsl) {
    float h = hsl.x, s = hsl.y, l = hsl.z;
    float nr = clamp(abs(h * 6.0 - 3.0) - 1.0, 0.0, 1.0);
    float ng = clamp(2.0 - abs(h * 6.0 - 2.0), 0.0, 1.0);
    float nb = clamp(2.0 - abs(h * 6.0 - 4.0), 0.0, 1.0);
    float chroma = (1.0 - abs(2.0 * l - 1.0)) * s;
    return (vec3(nr, ng, nb) - 0.5) * chroma + l;
}

// blend: the order of `BlendMode::ALL`.
vec3 fs_blend(int mode, float t, vec3 c1, vec3 c2) {
    float tm = 1.0 - t;
    if (mode == 0) return mix(c1, c2, t);
    if (mode == 1) return mix(c1, min(c1, c2), t);
    if (mode == 2) return mix(c1, c1 * c2, t);
    if (mode == 3) {
        vec3 o;
        for (int i = 0; i < 3; i++) {
            float tmp = tm + t * c2[i];
            o[i] = tmp <= 0.0 ? 0.0 : clamp(1.0 - (1.0 - c1[i]) / tmp, 0.0, 1.0);
        }
        return o;
    }
    if (mode == 4) return mix(c1, max(c1, c2), t);
    if (mode == 5) return vec3(1.0) - (vec3(tm) + t * (vec3(1.0) - c2)) * (vec3(1.0) - c1);
    if (mode == 6) {
        vec3 o;
        for (int i = 0; i < 3; i++) {
            float v = c1[i];
            if (v == 0.0) { o[i] = v; continue; }
            float tmp = 1.0 - t * c2[i];
            o[i] = tmp <= 0.0 ? 1.0 : min(v / tmp, 1.0);
        }
        return o;
    }
    if (mode == 7) return mix(c1, c1 + c2, t);
    if (mode == 8) {
        vec3 o;
        for (int i = 0; i < 3; i++) {
            float v = c1[i];
            o[i] = v < 0.5 ? v * (tm + 2.0 * t * c2[i]) : 1.0 - (tm + 2.0 * t * (1.0 - c2[i])) * (1.0 - v);
        }
        return o;
    }
    if (mode == 9) {
        vec3 scr = vec3(1.0) - (vec3(1.0) - c2) * (vec3(1.0) - c1);
        return c1 * tm + ((vec3(1.0) - c1) * c2 * c1 + c1 * scr) * t;
    }
    if (mode == 10) return c1 + (c2 * 2.0 - 1.0) * t;
    if (mode == 11) return mix(c1, abs(c1 - c2), t);
    if (mode == 12) return max(mix(c1, c1 + c2 - 2.0 * c1 * c2, t), vec3(0.0));
    if (mode == 13) return mix(c1, c1 - c2, t);
    if (mode == 14) {
        vec3 o;
        for (int i = 0; i < 3; i++) o[i] = c2[i] != 0.0 ? tm * c1[i] + t * c1[i] / c2[i] : c1[i];
        return o;
    }
    if (mode == 15) {
        vec3 hsv2 = fs_rgb_to_hsv(c2);
        if (hsv2.y != 0.0) { vec3 hsv = fs_rgb_to_hsv(c1); hsv.x = hsv2.x; return mix(c1, fs_hsv_to_rgb(hsv), t); }
        return c1;
    }
    if (mode == 16) {
        vec3 hsv = fs_rgb_to_hsv(c1);
        if (hsv.y != 0.0) { vec3 hsv2 = fs_rgb_to_hsv(c2); hsv.y = tm * hsv.y + t * hsv2.y; return fs_hsv_to_rgb(hsv); }
        return c1;
    }
    if (mode == 17) {
        vec3 hsv2 = fs_rgb_to_hsv(c2);
        if (hsv2.y != 0.0) { vec3 hsv = fs_rgb_to_hsv(c1); hsv.x = hsv2.x; hsv.y = hsv2.y; return mix(c1, fs_hsv_to_rgb(hsv), t); }
        return c1;
    }
    vec3 hsv = fs_rgb_to_hsv(c1);
    vec3 hsv2 = fs_rgb_to_hsv(c2);
    hsv.z = tm * hsv.z + t * hsv2.z;
    return fs_hsv_to_rgb(hsv);
}

vec3 fs_hue_saturation_value(vec3 color, float hue, float saturation, float value, float fac) {
    vec3 hsv = fs_rgb_to_hsv(color);
    hsv.x = fract(hsv.x + hue + 0.5);
    hsv.y = clamp(hsv.y * saturation, 0.0, 1.0);
    hsv.z *= value;
    return max(fac * fs_hsv_to_rgb(hsv) + (1.0 - fac) * color, vec3(0.0));
}

vec3 fs_gamma(vec3 color, float g) {
    if (g == 0.0) return vec3(1.0);
    vec3 o;
    for (int i = 0; i < 3; i++) o[i] = color[i] > 0.0 ? pow(color[i], g) : color[i];
    return o;
}

float fs_fresnel_dielectric_cos(float cosi, float eta) {
    float c = abs(cosi);
    float g = eta * eta - 1.0 + c * c;
    if (g > 0.0) {
        g = sqrt(g);
        float a = (g - c) / (g + c);
        float b = (c * (g + c) - 1.0) / (c * (g - c) + 1.0);
        return 0.5 * a * a * (1.0 + b * b);
    }
    return 1.0;
}

// A colour read as a number: its ACEScg luminance, as `Value::as_float` reads it.
float fs_luminance(vec3 c) { return dot(c, vec3(0.2722287, 0.6740818, 0.0536895)); }
