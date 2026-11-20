//! The first transport on the card (`docs/07` R2.5): camera rays, triangles,
//! one grey diffuse surface for everything, the environment sampled by its
//! own tables with next-event estimation and MIS, Russian roulette and the
//! bounce limit, as the processor's integrator has them (`transport.wgsl`).
//!
//! The card hands each sample back, radiance and the background its camera
//! ray saw, and the processor's [`Framebuffer`] accumulates them in sample
//! order, firefly rejection and all (`docs/07` D8), so the two renderers'
//! pictures differ only where their paths do.

use bytemuck::{Pod, Zeroable};
use tequila_core::spectrum::Spectrum;
use tequila_render_core::camera::Camera;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::integrator::PathResult;
use tequila_render_core::scene::SceneSnapshot;
use tequila_render_core::settings::IntegratorConfig;

use crate::device::GpuDevice;
use crate::environment::GpuEnvironment;
use crate::scene::GpuScene;
use crate::tables::Tables;

/// What a dispatch of `transport.wgsl` is told; `frame.wgsl` is its twin.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct FrameParams {
    pub camera_to_world: [[f32; 4]; 3],
    pub half_height: f32,
    pub half_width: f32,
    pub width: u32,
    pub height: u32,
    pub sample_base: u32,
    pub samples: u32,
    pub scene_seed: u32,
    pub max_bounces: u32,
    pub rr_start: u32,
    pub clamp_direct: f32,
    pub clamp_indirect: f32,
    pub shadow_far: f32,
    pub env_rotation: [[f32; 4]; 3],
    pub env_inverse_rotation: [[f32; 4]; 3],
    pub env_width: u32,
    pub env_height: u32,
    pub env_tiles_x: u32,
    pub env_tiles_y: u32,
    pub env_intensity: f32,
    pub env_weighted_sum: f32,
    pub env_infinite: u32,
    pub _pad0: u32,
    pub off_luminance: u32,
    pub off_cdf: u32,
    pub off_power: u32,
    pub off_rows: u32,
    pub off_choice: u32,
    pub off_tile_column: u32,
    pub off_tile_row: u32,
    pub off_row_start: u32,
}

/// What a render asks, as the processor's `RenderContext` has it.
#[derive(Clone, Debug)]
pub struct FrameSettings {
    pub camera: Camera,
    pub config: IntegratorConfig,
    pub scene_seed: u32,
    pub reject_fireflies: bool,
    pub clamp_direct: Option<f32>,
    pub clamp_indirect: Option<f32>,
}

/// Firefly rejection's ceiling on indirect light (`tequila-render-cpu`'s
/// `FIREFLY_CLAMP`).
pub const FIREFLY_CLAMP: f32 = 10.0;

impl FrameSettings {
    /// The processor's `renderer::clamps`: (direct, indirect).
    pub fn clamps(&self) -> (Option<f32>, Option<f32>) {
        let rejection = self.reject_fireflies.then_some(FIREFLY_CLAMP);
        match (self.clamp_direct, self.clamp_indirect) {
            (Some(direct), clamp) => (Some(direct), clamp.or(rejection)),
            (None, Some(clamp)) => (Some(clamp), Some(clamp)),
            (None, None) => (None, rejection),
        }
    }
}

/// How many samples a dispatch draws: a bounded buffer and a bounded time
/// for the card to run without the display.
const SAMPLES_PER_DISPATCH: u32 = 16;

/// A scene on one card, ready to render.
pub struct GpuRenderer {
    pub gpu: GpuDevice,
    tables: Tables,
    /// The scene's buffers and acceleration structures, which the bind
    /// groups read; kept for as long as the renderer is.
    pub scene: GpuScene,
    environment: GpuEnvironment,
    shadow_far: f32,
    pipeline: wgpu::ComputePipeline,
    scene_group: wgpu::BindGroup,
    environment_group: wgpu::BindGroup,
    frame_layout: wgpu::BindGroupLayout,
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None }
}

fn storage(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

impl GpuRenderer {
    /// Upload `scene` to `gpu`, which must have ray queries.
    pub fn new(gpu: GpuDevice, scene: &SceneSnapshot) -> GpuRenderer {
        let device = &gpu.device;
        let tables = Tables::new(device);
        let uploaded = GpuScene::upload(device, &gpu.queue, scene);
        let environment = GpuEnvironment::upload(device, &scene.environment);
        let shadow_far = scene.bounds().bounding_radius().max(1.0) * 1e4;

        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                entry(0, wgpu::BindingType::AccelerationStructure { vertex_return: false }),
                entry(1, storage(true)),
                entry(2, storage(true)),
                entry(3, storage(true)),
                entry(4, storage(true)),
                entry(5, storage(true)),
            ],
        });
        let environment_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("environment"),
            entries: &[entry(0, storage(true)), entry(1, storage(true)), entry(2, storage(true)), entry(3, storage(true))],
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                ),
                entry(1, storage(false)),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("transport"),
            bind_group_layouts: &[Some(&scene_layout), Some(&tables.layout), Some(&environment_layout), Some(&frame_layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("transport"),
            source: wgpu::ShaderSource::Wgsl(crate::shader::source("transport").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("transport"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("transport_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let scene_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout: &scene_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::AccelerationStructure(&uploaded.tlas) },
                wgpu::BindGroupEntry { binding: 1, resource: uploaded.positions.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: uploaded.normals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: uploaded.indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: uploaded.meshes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: uploaded.instances.as_entire_binding() },
            ],
        });
        let environment_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("environment"),
            layout: &environment_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: environment.image.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: environment.camera_image.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: environment.floats.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: environment.ints.as_entire_binding() },
            ],
        });
        GpuRenderer {
            gpu,
            tables,
            scene: uploaded,
            environment,
            shadow_far,
            pipeline,
            scene_group,
            environment_group,
            frame_layout,
        }
    }

    fn params(&self, settings: &FrameSettings, width: usize, height: usize, sample_base: u32, samples: u32) -> FrameParams {
        let aspect = width as f32 / height as f32;
        let half_height = (settings.camera.vertical_fov(aspect) * 0.5).tan();
        let m = settings.camera.camera_to_world.m;
        let (direct, indirect) = settings.clamps();
        let e = &self.environment.layout;
        FrameParams {
            camera_to_world: [m[0], m[1], m[2]],
            half_height,
            half_width: half_height * aspect,
            width: width as u32,
            height: height as u32,
            sample_base,
            samples,
            scene_seed: settings.scene_seed,
            max_bounces: settings.config.max_bounces,
            rr_start: settings.config.russian_roulette_start_bounce,
            clamp_direct: direct.unwrap_or(-1.0),
            clamp_indirect: indirect.unwrap_or(-1.0),
            shadow_far: self.shadow_far,
            env_rotation: e.rotation,
            env_inverse_rotation: e.inverse_rotation,
            env_width: e.width,
            env_height: e.height,
            env_tiles_x: e.tiles_x,
            env_tiles_y: e.tiles_y,
            env_intensity: e.intensity,
            env_weighted_sum: e.weighted_sum,
            env_infinite: e.infinite as u32,
            _pad0: 0,
            off_luminance: e.off_luminance,
            off_cdf: e.off_cdf,
            off_power: e.off_power,
            off_rows: e.off_rows,
            off_choice: e.off_choice,
            off_tile_column: e.off_tile_column,
            off_tile_row: e.off_tile_row,
            off_row_start: e.off_row_start,
        }
    }

    /// `samples` more samples into every pixel of `framebuffer`, as the
    /// processor's `render_pass` adds them: each pixel's next sample indices,
    /// accumulated in order. The framebuffer's pixels must all hold the same
    /// number of samples (a fresh one, or one only this has filled).
    pub fn render_pass(&self, framebuffer: &mut Framebuffer, settings: &FrameSettings, samples: u32) {
        let (width, height) = (framebuffer.width, framebuffer.height);
        let pixels = width * height;
        let device = &self.gpu.device;
        let start = framebuffer.pixels().first().map_or(0, |p| p.samples);
        let batch = SAMPLES_PER_DISPATCH.min(samples.max(1));
        let bytes = (pixels * batch as usize * 2 * 16) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("samples"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("samples, read back"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: std::mem::size_of::<FrameParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &self.frame_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
            ],
        });
        let mut done = 0u32;
        while done < samples {
            let count = batch.min(samples - done);
            let params = self.params(settings, width, height, start + done, count);
            self.gpu.queue.write_buffer(&uniform, 0, bytemuck::bytes_of(&params));
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("transport") });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("transport"), timestamp_writes: None });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.scene_group, &[]);
                pass.set_bind_group(1, &self.tables.bind_group, &[]);
                pass.set_bind_group(2, &self.environment_group, &[]);
                pass.set_bind_group(3, &frame_group, &[]);
                pass.dispatch_workgroups((width as u32).div_ceil(8), (height as u32).div_ceil(8), 1);
            }
            let used = (pixels * count as usize * 2 * 16) as u64;
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, used);
            self.gpu.queue.submit(Some(encoder.finish()));
            readback.slice(..used).map_async(wgpu::MapMode::Read, |r| r.expect("the samples map"));
            self.gpu.wait();
            {
                let view = readback.slice(..used).get_mapped_range().expect("the samples are mapped");
                let values: &[[f32; 4]] = bytemuck::cast_slice(&view[..]);
                let reject = settings.reject_fireflies;
                for (index, pixel) in framebuffer.pixels_mut().iter_mut().enumerate() {
                    for i in 0..count as usize {
                        let at = (i * pixels + index) * 2;
                        let (r, b) = (values[at], values[at + 1]);
                        let result = PathResult {
                            radiance: Spectrum { r: r[0], g: r[1], b: r[2] },
                            background: Spectrum { r: b[0], g: b[1], b: b[2] },
                            ..Default::default()
                        };
                        pixel.accumulate_with(&result, reject, None);
                    }
                }
            }
            readback.unmap();
            done += count;
        }
    }
}
