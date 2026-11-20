// What a frame of the first transport is told (transport.rs's FrameParams),
// at @group(3) @binding(0), and where its samples go, @binding(1).

struct FrameParams {
    // Camera-to-world, row-major 3 x 4 (Camera::camera_to_world).
    camera_to_world: array<vec4<f32>, 3>,
    // tan(vertical fov / 2), and that times the aspect.
    half_height: f32,
    half_width: f32,
    width: u32,
    height: u32,
    // The first sample index of this dispatch, and how many it draws.
    sample_base: u32,
    samples: u32,
    scene_seed: u32,
    max_bounces: u32,
    // Russian roulette from this bounce on.
    rr_start: u32,
    // Ceilings on a contribution's summed red, green and blue; below zero,
    // none.
    clamp_direct: f32,
    clamp_indirect: f32,
    // How far past a shading point a shadow ray towards the sky aims.
    shadow_far: f32,
    env_rotation: array<vec4<f32>, 3>,
    env_inverse_rotation: array<vec4<f32>, 3>,
    env_width: u32,
    env_height: u32,
    env_tiles_x: u32,
    env_tiles_y: u32,
    env_intensity: f32,
    env_weighted_sum: f32,
    env_infinite: u32,
    // Bit 0: add the samples up on the card (frame_pixels) rather than hand
    // each back (frame_samples). Bit 1: firefly rejection.
    mode: u32,
    // Offsets into env_f32 and env_u32 (environment.wgsl).
    off_luminance: u32,
    off_cdf: u32,
    off_power: u32,
    off_rows: u32,
    off_choice: u32,
    off_tile_column: u32,
    off_tile_row: u32,
    off_row_start: u32,
    // Path regularization (IntegratorConfig): from this bounce on, every
    // reflection lobe at least this rough.
    regularization_start: u32,
    regularization_roughness: f32,
    _pad1: u32,
    _pad2: u32,
};

@group(3) @binding(0) var<uniform> frame: FrameParams;
// Two vec4s a sample: radiance (and in w, 1 if the camera ray met
// geometry), then the background a camera ray saw.
@group(3) @binding(1) var<storage, read_write> frame_samples: array<vec4<f32>>;

// Framebuffer's Pixel, as far as this transport fills it.
struct PixelSums {
    // Radiance, and in w the energy firefly rejection took off.
    sum: vec4<f32>,
    // Background, and in w the coverage.
    background: vec4<f32>,
    // Squared luminance, in two floats for the processor's one f64: the
    // sum, and Kahan's compensation, which it is over by.
    squares_high: f32,
    squares_low: f32,
    samples: u32,
    _pad: u32,
};

@group(3) @binding(2) var<storage, read_write> frame_pixels: array<PixelSums>;

fn luminance(c: vec3<f32>) -> f32 {
    return 0.2722287 * c.x + 0.6740818 * c.y + 0.0536895 * c.z;
}

// Pixel::accumulate_with for a sample with no hue, no wavelength and no
// clamp: firefly rejection (k = 8 deviations, from the 16th sample on)
// judging only what the path found beyond the background.
fn accumulate(pixel: u32, radiance_in: vec3<f32>, background: vec3<f32>, coverage: f32) {
    var p = frame_pixels[pixel];
    var radiance = radiance_in;
    if ((frame.mode & 2u) != 0u && p.samples >= 16u) {
        let sampled = max(radiance - background, vec3<f32>(0.0));
        let lum = luminance(sampled);
        let n = f32(p.samples);
        let mean = luminance(p.sum.xyz * (1.0 / n));
        // Kahan's compensation is what the sum is over by.
        let squares = p.squares_high - p.squares_low;
        let deviation = sqrt(max(squares / n - mean * mean, 0.0));
        let limit = mean + 8.0 * deviation;
        if (lum > limit && limit < 3.4e38) {
            p.sum.w += lum - limit;
            radiance = radiance - sampled * (1.0 - limit / max(lum, 1e-9));
        }
    }
    p.sum = vec4<f32>(p.sum.xyz + radiance, p.sum.w);
    p.background = vec4<f32>(p.background.xyz + background, p.background.w + clamp(coverage, 0.0, 1.0));
    // Kahan: the low word keeps what the high word's rounding lost.
    let l = luminance(radiance);
    let y = l * l - p.squares_low;
    let t = p.squares_high + y;
    p.squares_low = (t - p.squares_high) - y;
    p.squares_high = t;
    p.samples += 1u;
    frame_pixels[pixel] = p;
}
