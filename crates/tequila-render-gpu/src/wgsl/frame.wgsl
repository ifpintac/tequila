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
    _pad0: u32,
    // Offsets into env_f32 and env_u32 (environment.wgsl).
    off_luminance: u32,
    off_cdf: u32,
    off_power: u32,
    off_rows: u32,
    off_choice: u32,
    off_tile_column: u32,
    off_tile_row: u32,
    off_row_start: u32,
};

@group(3) @binding(0) var<uniform> frame: FrameParams;
// Two vec4s a sample: radiance, then the background a camera ray saw.
@group(3) @binding(1) var<storage, read_write> frame_samples: array<vec4<f32>>;
