//! The environment light on the card: its images and its sampling tables as
//! `Environment::tables` hands them over, uploaded unchanged, so the card
//! draws from the very sky the processor draws from (`environment.wgsl`):
//! the lights' and, for the shadow catcher, the floor's. And a backplate's
//! picture, after the camera's image in its buffer.

use tequila_render_core::environment::{Background, Environment, EnvironmentTables};
use tequila_texture::Image;
use wgpu::util::DeviceExt;

/// Where each table starts in the two packed buffers, and the numbers
/// [`crate::transport::FrameParams`] carries about them.
#[derive(Clone, Copy, Debug, Default)]
pub struct EnvLayout {
    pub width: u32,
    pub height: u32,
    pub tiles_x: u32,
    pub tiles_y: u32,
    pub intensity: f32,
    pub weighted_sum: f32,
    pub infinite: bool,
    pub rotation: [[f32; 4]; 3],
    pub inverse_rotation: [[f32; 4]; 3],
    pub off_luminance: u32,
    pub off_cdf: u32,
    pub off_power: u32,
    pub off_rows: u32,
    pub off_choice: u32,
    pub off_tile_column: u32,
    pub off_tile_row: u32,
    pub off_row_start: u32,
    /// The rows' CDF a surface not facing straight up draws its light from,
    /// and the one weighted towards straight up a floor draws from, with its
    /// sum (`Environment::sample_irradiance`, the shadow catcher's).
    pub off_marginal: u32,
    pub off_upward: u32,
    pub upward_sum: f32,
    /// A backplate's picture (`Background::Backplate`), one `vec4` a texel
    /// from `plate_offset` in the camera's image buffer; 0 by 0 for none.
    pub plate_offset: u32,
    pub plate_width: u32,
    pub plate_height: u32,
}

pub struct GpuEnvironment {
    pub image: wgpu::Buffer,
    pub camera_image: wgpu::Buffer,
    pub floats: wgpu::Buffer,
    pub ints: wgpu::Buffer,
    pub layout: EnvLayout,
}

/// An image as RGBA, one `vec4` a texel.
fn texels(image: &Image) -> Vec<[f32; 4]> {
    let channels = image.channels.max(1);
    (0..image.width * image.height)
        .map(|i| {
            let p = &image.data[i * channels..i * channels + channels];
            if channels >= 3 {
                [p[0], p[1], p[2], 0.0]
            } else {
                [p[0], p[0], p[0], 0.0]
            }
        })
        .collect()
}

impl GpuEnvironment {
    /// `environment`'s images and tables, and `background`'s backplate if
    /// it is one.
    pub fn upload(device: &wgpu::Device, environment: &Environment, background: &Background) -> GpuEnvironment {
        let t: EnvironmentTables = environment.tables();
        let mut floats: Vec<f32> = Vec::new();
        let mut push_f = |values: &[f32]| {
            let at = floats.len() as u32;
            floats.extend_from_slice(values);
            at
        };
        let off_luminance = push_f(t.luminance);
        let off_cdf = push_f(t.conditional_cdf);
        let off_power = push_f(t.power);
        let off_rows = push_f(t.rows);
        let off_choice = push_f(t.choice);
        let off_marginal = push_f(t.marginal_cdf);
        let off_upward = push_f(t.upward_cdf);
        let mut ints: Vec<u32> = Vec::new();
        let mut push_u = |values: &mut dyn Iterator<Item = u32>| {
            let at = ints.len() as u32;
            ints.extend(values);
            at
        };
        let off_tile_column = push_u(&mut t.tile_of_column.iter().map(|&v| v as u32));
        let off_tile_row = push_u(&mut t.tile_of_row.iter().map(|&v| v as u32));
        let off_row_start = push_u(&mut t.row_start.iter().map(|&v| v as u32));
        if floats.is_empty() {
            floats.push(0.0);
        }
        if ints.is_empty() {
            ints.push(0);
        }
        let rows = |m: &tequila_core::math::Mat4| [m.m[0], m.m[1], m.m[2]];
        let mut layout = EnvLayout {
            width: t.width as u32,
            height: t.height as u32,
            tiles_x: t.tiles_x as u32,
            tiles_y: t.tiles_y as u32,
            intensity: t.intensity,
            weighted_sum: t.weighted_sum,
            infinite: t.infinite,
            rotation: rows(&t.rotation),
            inverse_rotation: rows(&t.inverse_rotation),
            off_luminance,
            off_cdf,
            off_power,
            off_rows,
            off_choice,
            off_tile_column,
            off_tile_row,
            off_row_start,
            off_marginal,
            off_upward,
            upward_sum: t.upward_sum,
            plate_offset: 0,
            plate_width: 0,
            plate_height: 0,
        };
        let storage = wgpu::BufferUsages::STORAGE;
        let init = |label: &str, contents: &[u8]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage: storage })
        };
        let lit = texels(t.image);
        let mut seen = texels(t.camera_image);
        if let Background::Backplate { image } = background {
            layout.plate_offset = seen.len() as u32;
            layout.plate_width = image.width as u32;
            layout.plate_height = image.height as u32;
            seen.extend(texels(image));
        }
        GpuEnvironment {
            image: init("environment", bytemuck::cast_slice(&lit)),
            camera_image: init("environment, seen", bytemuck::cast_slice(&seen)),
            floats: init("environment tables", bytemuck::cast_slice(&floats)),
            ints: init("environment tiles", bytemuck::cast_slice(&ints)),
            layout,
        }
    }
}
