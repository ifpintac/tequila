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
    // The shadow catcher's tables in env_f32 (environment.wgsl's
    // env_sample_irradiance): the rows' CDF, and the one weighted towards
    // straight up, with its sum.
    off_marginal: u32,
    off_upward: u32,
    env_upward_sum: f32,
    // IntegratorConfig::shadows, and ::ground_illumination (a floor a bounce
    // meets is shaded as its material rather than passed), 1 for on.
    shadows: u32,
    ground_illumination: u32,
    // How many lamps the scene has (lamps.wgsl), whether its light
    // hierarchy chooses among them rather than its alias table, and
    // lights.rs's LAMPS_* flags.
    light_count: u32,
    light_hierarchy: u32,
    lamp_flags: u32,
    // IntegratorConfig::pixel_spread: a footprint is the hit's distance times
    // it, for a material's program and its bump.
    pixel_spread: f32,
    // IntegratorConfig::max_transmission_bounces: how many times a path may
    // go through a surface.
    max_transmission_bounces: u32,
    // IntegratorConfig::shadow_glass: the clear glass shadow rays pass
    // (GLASS_SOLID, GLASS_CLEAR, GLASS_UNBENT).
    shadow_glass: u32,
    // SCENE_TRANSMITS: some material transmits (SceneSnapshot::
    // has_transmissive), so a path starts by asking which media it is in;
    // SCENE_PANES: some material is clear glass a shadow ray passes;
    // SCENE_CUTOUTS: some material is cut out (SceneSnapshot::has_cutouts);
    // SCENE_CUT_PROGRAMS: a program on the card drives some opacity;
    // SCENE_EMITTERS: the glowing triangles are lights (emitters.wgsl);
    // SCENE_LIGHT_MATERIALS: IntegratorConfig::light_source_materials;
    // SCENE_BEVEL: some material rounds its edges (edge_radius).
    glass_flags: u32,
    // The rows this dispatch draws, row_base and every row_stride-th after
    // it: render_pass draws a big frame in interleaved sets of rows, each
    // within the time a display allows the card and each as costly as any
    // other. A whole frame is row 0 at a stride of 1.
    row_base: u32,
    row_stride: u32,
};

// FrameParams::shadow_glass (settings.rs's ShadowGlass).
const GLASS_SOLID: u32 = 0u;
const GLASS_CLEAR: u32 = 1u;
const GLASS_UNBENT: u32 = 2u;

// FrameParams::glass_flags.
const SCENE_TRANSMITS: u32 = 1u;
const SCENE_PANES: u32 = 2u;
const SCENE_CUTOUTS: u32 = 4u;
const SCENE_EMITTERS: u32 = 8u;
const SCENE_LIGHT_MATERIALS: u32 = 16u;
const SCENE_CUT_PROGRAMS: u32 = 32u;
const SCENE_BEVEL: u32 = 64u;

// FrameParams::lamp_flags (lights.rs's LAMPS_*): a bounce off a sharp
// enough reflection has a density to see by; a lamp a seeing bounce meets;
// lamps shown in mirrors rather than by their stand-ins; one of the lamps a
// bounce meets lights nothing diffusely.
const LAMPS_SEEN: u32 = 1u;
const LAMPS_MET: u32 = 2u;
const LAMPS_IN_MIRRORS: u32 = 4u;
const LAMPS_FOR_REFLECTIONS: u32 = 8u;
// Shadow rays pass all clear glass and there is some, so a bounce that sees
// meets glass that bends light as glass (the integrator's meets_glass).
const LAMPS_MEETS_GLASS: u32 = 16u;

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
