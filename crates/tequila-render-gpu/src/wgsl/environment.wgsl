// The environment light: tequila_render_core::environment's radiance,
// background, sample_facing and pdf_facing, reading its own images and
// tables as Environment::tables hands them over (environment.rs here), at
// @group(2).
//#include scene
//#include frame

@group(2) @binding(0) var<storage, read> env_image: array<vec4<f32>>;
@group(2) @binding(1) var<storage, read> env_camera_image: array<vec4<f32>>;
@group(2) @binding(2) var<storage, read> env_f32: array<f32>;
@group(2) @binding(3) var<storage, read> env_u32: array<u32>;

const PI: f32 = 3.14159265358979323846;
const TAU: f32 = 6.28318530717958647692;
const FRAC_1_PI: f32 = 0.318309886183790671538;
// 1 - f32::EPSILON, and 1 - f32::EPSILON / 2.
const ONE_MINUS_EPS: f32 = 0.99999988079071044921875;

// Vec3::normalize: times the reciprocal of the length.
fn normalized(v: vec3<f32>) -> vec3<f32> {
    return v * (1.0 / sqrt(dot(v, v)));
}

fn rem_euclid1(x: f32) -> f32 {
    let r = x - trunc(x);
    return select(r, r + 1.0, r < 0.0);
}

fn direction_to_equirect(d: vec3<f32>) -> vec2<f32> {
    let theta = acos(clamp(d.y, -1.0, 1.0));
    let phi = atan2(d.x, -d.z);
    return vec2<f32>(rem_euclid1(phi / TAU + 0.5), theta / PI);
}

fn equirect_to_direction(uv: vec2<f32>) -> vec3<f32> {
    let phi = (uv.x - 0.5) * TAU;
    let theta = uv.y * PI;
    let s = sin(theta);
    return vec3<f32>(s * sin(phi), cos(theta), -s * cos(phi));
}

fn env_texel(camera: bool, x: u32, y: u32) -> vec3<f32> {
    let i = y * frame.env_width + x;
    if (camera) {
        return env_camera_image[i].xyz;
    }
    return env_image[i].xyz;
}

fn lerp3(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    return a * (1.0 - t) + b * t;
}

// Image::sample_bilinear_wrap_u.
fn env_bilinear(camera: bool, uv: vec2<f32>) -> vec3<f32> {
    let w = frame.env_width;
    let h = frame.env_height;
    let fx = rem_euclid1(uv.x) * f32(w) - 0.5;
    let fy = clamp(uv.y, 0.0, 1.0) * f32(h) - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let xi = i32(x0);
    let yi = i32(y0);
    let wi = i32(w);
    let x0i = u32(((xi % wi) + wi) % wi);
    let x1i = u32((((xi + 1) % wi) + wi) % wi);
    let y0i = u32(clamp(yi, 0, i32(h) - 1));
    let y1i = u32(clamp(yi + 1, 0, i32(h) - 1));
    let c00 = env_texel(camera, x0i, y0i);
    let c10 = env_texel(camera, x1i, y0i);
    let c01 = env_texel(camera, x0i, y1i);
    let c11 = env_texel(camera, x1i, y1i);
    return lerp3(lerp3(c00, c10, tx), lerp3(c01, c11, tx), ty);
}

// Background::radiance's backplate: Image::sample_bilinear_wrap_u of the
// plate at the film point, each coordinate clamped to [0, 1] first (so the
// right edge, u = 1, wraps to the left's texels, as the processor's does).
fn plate_bilinear(film: vec2<f32>) -> vec3<f32> {
    let w = frame.plate_width;
    let h = frame.plate_height;
    if (w == 0u || h == 0u) {
        return vec3<f32>(0.0);
    }
    let fx = rem_euclid1(clamp(film.x, 0.0, 1.0)) * f32(w) - 0.5;
    let fy = clamp(film.y, 0.0, 1.0) * f32(h) - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let xi = i32(x0);
    let yi = i32(y0);
    let wi = i32(w);
    let x0i = u32(((xi % wi) + wi) % wi);
    let x1i = u32((((xi + 1) % wi) + wi) % wi);
    let y0i = u32(clamp(yi, 0, i32(h) - 1));
    let y1i = u32(clamp(yi + 1, 0, i32(h) - 1));
    let at = frame.plate_offset;
    let c00 = env_camera_image[at + y0i * w + x0i].xyz;
    let c10 = env_camera_image[at + y0i * w + x1i].xyz;
    let c01 = env_camera_image[at + y1i * w + x0i].xyz;
    let c11 = env_camera_image[at + y1i * w + x1i].xyz;
    return lerp3(lerp3(c00, c10, tx), lerp3(c01, c11, tx), ty);
}

// Environment::radiance (the lit image), or ::background_radiance_from (what
// the camera sees), the dome infinitely far.
fn env_radiance(camera: bool, world_direction: vec3<f32>) -> vec3<f32> {
    if (frame.env_intensity == 0.0) {
        return vec3<f32>(0.0);
    }
    let local = rows_vector(frame.env_inverse_rotation, world_direction);
    return env_bilinear(camera, direction_to_equirect(local)) * frame.env_intensity;
}

fn facing_bin(n: vec3<f32>) -> u32 {
    let ax = abs(n.x);
    let ay = abs(n.y);
    let az = abs(n.z);
    var face: u32;
    var a: f32;
    var b: f32;
    var major: f32;
    if (ax >= ay && ax >= az) {
        face = select(1u, 0u, n.x >= 0.0); a = n.y; b = n.z; major = ax;
    } else if (ay >= az) {
        face = select(3u, 2u, n.y >= 0.0); a = n.x; b = n.z; major = ay;
    } else {
        face = select(5u, 4u, n.z >= 0.0); a = n.x; b = n.y; major = az;
    }
    major = max(major, 1e-12);
    let ca = min(u32(max((a / major + 1.0) * 0.5 * 8.0, 0.0)), 7u);
    let cb = min(u32(max((b / major + 1.0) * 0.5 * 8.0, 0.0)), 7u);
    return (face * 8u + ca) * 8u + cb;
}

// find_interval over env_f32[offset .. offset + count + 1].
fn find_interval(offset: u32, count: u32, u: f32) -> u32 {
    var lo = 0u;
    var hi = count;
    while (lo + 1u < hi) {
        let mid = (lo + hi) / 2u;
        if (env_f32[offset + mid] <= u) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    return min(lo, count - 1u);
}

fn segment_offset(offset: u32, i: u32, u: f32) -> f32 {
    let c0 = env_f32[offset + i];
    let width = env_f32[offset + i + 1u] - c0;
    if (width > 0.0) {
        return clamp((u - c0) / width, 0.0, ONE_MINUS_EPS);
    }
    return 0.5;
}

fn tiles() -> u32 {
    return frame.env_tiles_x * frame.env_tiles_y;
}

// The facing's tile chances start here in env_f32.
fn choices_of(normal: vec3<f32>) -> u32 {
    let bin = facing_bin(rows_vector(frame.env_inverse_rotation, normal));
    return frame.off_choice + bin * (tiles() + 1u);
}

// Environment::pdf_facing, the dome infinite.
fn env_pdf_facing(world_direction: vec3<f32>, normal: vec3<f32>) -> f32 {
    if (frame.env_weighted_sum <= 0.0) {
        return 0.0;
    }
    let choices = choices_of(normal);
    let local = rows_vector(frame.env_inverse_rotation, world_direction);
    let uv = direction_to_equirect(local);
    let w = frame.env_width;
    let h = frame.env_height;
    let x = min(u32(max(uv.x * f32(w), 0.0)), w - 1u);
    let y = min(u32(max(uv.y * f32(h), 0.0)), h - 1u);
    let t = env_u32[frame.off_tile_row + y] * frame.env_tiles_x + env_u32[frame.off_tile_column + x];
    let chance = env_f32[choices + t + 1u] - env_f32[choices + t];
    let power = env_f32[frame.off_power + t];
    if (!(chance > 0.0 && power > 0.0)) {
        return 0.0;
    }
    return chance * env_f32[frame.off_luminance + y * w + x] * f32(w * h) / (((power * 2.0) * PI) * PI);
}

struct EnvSample {
    ok: bool,
    direction: vec3<f32>,
    radiance: vec3<f32>,
    pdf: f32,
};

// Environment::sample_facing, the dome infinite.
fn env_sample_facing(normal: vec3<f32>, u: vec2<f32>) -> EnvSample {
    var none: EnvSample;
    none.ok = false;
    if (frame.env_weighted_sum <= 0.0) {
        return none;
    }
    let choices = choices_of(normal);
    let n_tiles = tiles();
    if (!(env_f32[choices + n_tiles] > 0.0)) {
        return none;
    }
    let t = find_interval(choices, n_tiles, u.x);
    let ux = segment_offset(choices, t, u.x);
    let w = frame.env_width;
    let h = frame.env_height;
    let tx = t % frame.env_tiles_x;
    let ty = t / frame.env_tiles_x;
    let x0 = tx * w / frame.env_tiles_x;
    let x1 = (tx + 1u) * w / frame.env_tiles_x;
    let y0 = ty * h / frame.env_tiles_y;
    let rows = frame.off_rows + env_u32[frame.off_row_start + t];
    let row_count = (ty + 1u) * h / frame.env_tiles_y - y0;
    let total = env_f32[rows + row_count];
    if (!(total > 0.0)) {
        return none;
    }
    let along = u.y * total;
    let ry = find_interval(rows, row_count, along);
    let dy = segment_offset(rows, ry, along);
    let y = y0 + ry;
    let row = frame.off_cdf + y * (w + 1u);
    let across = env_f32[row + x0] + ux * (env_f32[row + x1] - env_f32[row + x0]);
    let x = clamp(find_interval(row, w, across), x0, x1 - 1u);
    let dx = segment_offset(row, x, across);
    let uv = vec2<f32>((f32(x) + dx) / f32(w), (f32(y) + dy) / f32(h));
    let direction = normalized(rows_vector(frame.env_rotation, equirect_to_direction(uv)));
    let pdf = env_pdf_facing(direction, normal);
    if (!(pdf > 0.0)) {
        return none;
    }
    return EnvSample(true, direction, env_radiance(false, direction), pdf);
}

// Environment::pdf, the dome infinite: the whole sky's density, as a
// surface lit from both sides draws it.
fn env_pdf_whole(world_direction: vec3<f32>) -> f32 {
    if (frame.env_weighted_sum <= 0.0) {
        return 0.0;
    }
    let local = rows_vector(frame.env_inverse_rotation, world_direction);
    let uv = direction_to_equirect(local);
    let w = frame.env_width;
    let h = frame.env_height;
    let x = min(u32(max(uv.x * f32(w), 0.0)), w - 1u);
    let y = min(u32(max(uv.y * f32(h), 0.0)), h - 1u);
    return env_f32[frame.off_luminance + y * w + x] * f32(w * h) / (((frame.env_weighted_sum * 2.0) * PI) * PI);
}

// Environment::pdf_facing with the facing optional: a zero `normal` is the
// whole sky (None).
fn env_pdf_sky(world_direction: vec3<f32>, normal: vec3<f32>) -> f32 {
    if (all(normal == vec3<f32>(0.0))) {
        return env_pdf_whole(world_direction);
    }
    return env_pdf_facing(world_direction, normal);
}

// Environment::sample_facing with the facing optional: a zero `normal`
// draws from the whole sky (Environment::sample: a row by the marginal, a
// column within it by its own CDF), else only what a surface facing it can
// see (env_sample_facing). One call site for both, for the kernel's size.
fn env_sample_sky(normal: vec3<f32>, u: vec2<f32>) -> EnvSample {
    if (!all(normal == vec3<f32>(0.0))) {
        return env_sample_facing(normal, u);
    }
    var none: EnvSample;
    none.ok = false;
    if (frame.env_weighted_sum <= 0.0) {
        return none;
    }
    let w = frame.env_width;
    let h = frame.env_height;
    let y = find_interval(frame.off_marginal, h, u.y);
    let dy = segment_offset(frame.off_marginal, y, u.y);
    let row = frame.off_cdf + y * (w + 1u);
    let x = find_interval(row, w, u.x);
    let dx = segment_offset(row, x, u.x);
    let uv = vec2<f32>((f32(x) + dx) / f32(w), (f32(y) + dy) / f32(h));
    let direction = normalized(rows_vector(frame.env_rotation, equirect_to_direction(uv)));
    let pdf = env_pdf_whole(direction);
    if (!(pdf > 0.0)) {
        return none;
    }
    return EnvSample(true, direction, env_radiance(false, direction), pdf);
}

// Environment::faces_up: a surface facing straight up, near enough to be
// drawn for exactly, under a dome infinitely far.
fn env_faces_up(normal: vec3<f32>) -> bool {
    return frame.env_infinite != 0u && rows_vector(frame.env_inverse_rotation, normal).y >= 1.0 - 1e-5;
}

// Environment::irradiance_bound, the dome infinite (its spread bound 1).
fn env_irradiance_bound(normal: vec3<f32>) -> f32 {
    if (frame.env_weighted_sum <= 0.0) {
        return 0.0;
    }
    let per_texel = frame.env_intensity * 2.0 * PI * PI / f32(frame.env_width * frame.env_height);
    if (env_faces_up(normal)) {
        return per_texel * frame.env_upward_sum;
    }
    return per_texel * frame.env_weighted_sum;
}

struct IrradianceSample {
    ok: bool,
    direction: vec3<f32>,
    keep: f32,
};

// Environment::row_cosines: the cosines of the latitudes at the top and the
// bottom of row y.
fn env_row_cosines(y: u32) -> vec2<f32> {
    let h = f32(frame.env_height);
    return vec2<f32>(cos(f32(y) / h * PI), cos(f32(y + 1u) / h * PI));
}

// Environment::direction_at: the world direction dx across column x at
// latitude cos_theta.
fn env_direction_at(x: u32, dx: f32, cos_theta: f32) -> vec3<f32> {
    let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 0.0));
    let phi = ((f32(x) + dx) / f32(frame.env_width) - 0.5) * TAU;
    let local = vec3<f32>(sin_theta * sin(phi), cos_theta, -sin_theta * cos(phi));
    return normalized(rows_vector(frame.env_rotation, local));
}

// Environment::sample_irradiance, the dome infinite: a direction towards the
// sky for the light onto a surface facing `normal`, and the chance of keeping
// it. A floor facing straight up draws rows by the light they bring onto it
// (the upward CDF) and keeps every draw.
fn env_sample_irradiance(normal: vec3<f32>, u: vec2<f32>) -> IrradianceSample {
    var none: IrradianceSample;
    none.ok = false;
    if (frame.env_weighted_sum <= 0.0) {
        return none;
    }
    let w = frame.env_width;
    let h = frame.env_height;
    let up = env_faces_up(normal);
    if (up && frame.env_upward_sum <= 0.0) {
        return none;
    }
    // texel_at: a row by the marginal, a column within it by its own CDF.
    let marginal = select(frame.off_marginal, frame.off_upward, up);
    let y = find_interval(marginal, h, u.y);
    let dy = segment_offset(marginal, y, u.y);
    let row = frame.off_cdf + y * (w + 1u);
    let x = find_interval(row, w, u.x);
    let dx = segment_offset(row, x, u.x);
    let c = env_row_cosines(y);
    if (up) {
        let top = c.x;
        let bottom = max(c.y, 0.0);
        let cos_theta = sqrt(max(bottom * bottom + dy * (top * top - bottom * bottom), 0.0));
        return IrradianceSample(true, env_direction_at(x, dx, cos_theta), 1.0);
    }
    let direction = env_direction_at(x, dx, c.x + dy * (c.y - c.x));
    return IrradianceSample(true, direction, max(dot(normal, direction), 0.0));
}
