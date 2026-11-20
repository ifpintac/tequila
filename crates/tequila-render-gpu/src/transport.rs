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
    /// Bit 0: add the samples up on the card; bit 1: firefly rejection.
    pub mode: u32,
    pub off_luminance: u32,
    pub off_cdf: u32,
    pub off_power: u32,
    pub off_rows: u32,
    pub off_choice: u32,
    pub off_tile_column: u32,
    pub off_tile_row: u32,
    pub off_row_start: u32,
    pub regularization_start: u32,
    pub regularization_roughness: f32,
    pub _pad1: u32,
    pub _pad2: u32,
}

/// A pixel's sums on the card (`frame.wgsl`'s `PixelSums`): what of
/// [`Framebuffer`]'s `Pixel` this transport fills.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct PixelSums {
    /// Radiance, and in `[3]` the energy firefly rejection took off.
    pub sum: [f32; 4],
    /// Background, and in `[3]` the coverage.
    pub background: [f32; 4],
    /// Squared luminance: the sum and Kahan's compensation, which the sum is
    /// over by.
    pub squares_high: f32,
    pub squares_low: f32,
    pub samples: u32,
    pub _pad: u32,
}

const MODE_ON_CARD: u32 = 1;
const MODE_REJECT: u32 = 2;

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
    /// Add the samples up on the card, firefly rejection and all, and bring
    /// back each pixel's sums once a pass: the fast way, and the default.
    /// Off, each sample comes back and the processor's `Framebuffer` adds it
    /// up, exactly as it adds up its own (the parity test).
    pub accumulate_on_card: bool,
    tables: Tables,
    /// The scene's buffers and acceleration structures, which the bind
    /// groups read; kept for as long as the renderer is.
    pub scene: GpuScene,
    environment: GpuEnvironment,
    shadow_far: f32,
    /// The scene's materials, which the scene's bind group reads.
    pub materials: wgpu::Buffer,
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
        // The materials as data. One the card's stack cannot render yet
        // (glass, skin, film, flakes) renders as its opaque part, and says so.
        let records: Vec<crate::material::GpuMaterial> = scene.materials.iter().map(crate::material::GpuMaterial::from_material).collect();
        let unsupported = scene.materials.iter().filter(|m| !crate::material::GpuMaterial::supports(m)).count();
        if unsupported > 0 {
            eprintln!("  {unsupported} material(s) render as their opaque part on the graphics card for now (docs/07 R3)");
        }
        let records = if records.is_empty() { vec![crate::material::GpuMaterial::default()] } else { records };
        let materials = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("materials"),
                contents: bytemuck::cast_slice(&records),
                usage: wgpu::BufferUsages::STORAGE,
            },
        );

        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                entry(0, wgpu::BindingType::AccelerationStructure { vertex_return: false }),
                entry(1, storage(true)),
                entry(2, storage(true)),
                entry(3, storage(true)),
                entry(4, storage(true)),
                entry(5, storage(true)),
                entry(6, storage(true)),
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
                entry(2, storage(false)),
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
                wgpu::BindGroupEntry { binding: 6, resource: materials.as_entire_binding() },
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
            accumulate_on_card: true,
            tables,
            scene: uploaded,
            environment,
            shadow_far,
            materials,
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
            mode: if self.accumulate_on_card { MODE_ON_CARD } else { 0 } | if settings.reject_fireflies { MODE_REJECT } else { 0 },
            off_luminance: e.off_luminance,
            off_cdf: e.off_cdf,
            off_power: e.off_power,
            off_rows: e.off_rows,
            off_choice: e.off_choice,
            off_tile_column: e.off_tile_column,
            off_tile_row: e.off_tile_row,
            off_row_start: e.off_row_start,
            regularization_start: settings.config.regularization_start_bounce,
            regularization_roughness: settings.config.regularization_roughness,
            _pad1: 0,
            _pad2: 0,
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
        let on_card = self.accumulate_on_card;
        // Each sample back, when the processor adds them up; else a word.
        let sample_bytes = if on_card { 16 } else { (pixels * batch as usize * 2 * 16) as u64 };
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("samples"),
            size: sample_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        // The pixels' sums so far, when the card adds them up; else a pixel.
        let sums: Vec<PixelSums> = if on_card {
            framebuffer
                .pixels()
                .iter()
                .map(|p| {
                    let high = p.sum_squared_luminance as f32;
                    PixelSums {
                        sum: [p.sum.r, p.sum.g, p.sum.b, p.clamped_energy],
                        background: [p.background_sum.r, p.background_sum.g, p.background_sum.b, p.coverage_sum],
                        squares_high: high,
                        squares_low: (high as f64 - p.sum_squared_luminance) as f32,
                        samples: p.samples,
                        _pad: 0,
                    }
                })
                .collect()
        } else {
            vec![PixelSums::default()]
        };
        let pixel_bytes = (sums.len() * std::mem::size_of::<PixelSums>()) as u64;
        let pixel_buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("pixels"),
                contents: bytemuck::cast_slice(&sums),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            },
        );
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read back"),
            size: sample_bytes.max(pixel_bytes),
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
                wgpu::BindGroupEntry { binding: 2, resource: pixel_buffer.as_entire_binding() },
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
            if on_card {
                self.gpu.queue.submit(Some(encoder.finish()));
                done += count;
                continue;
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
                            coverage: r[3],
                            escaped: r[3] == 0.0,
                            ..Default::default()
                        };
                        pixel.accumulate_with(&result, reject, None);
                    }
                }
            }
            readback.unmap();
            done += count;
        }
        if on_card {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("read back") });
            encoder.copy_buffer_to_buffer(&pixel_buffer, 0, &readback, 0, pixel_bytes);
            self.gpu.queue.submit(Some(encoder.finish()));
            readback.slice(..pixel_bytes).map_async(wgpu::MapMode::Read, |r| r.expect("the pixels map"));
            self.gpu.wait();
            {
                let view = readback.slice(..pixel_bytes).get_mapped_range().expect("the pixels are mapped");
                let back: &[PixelSums] = bytemuck::cast_slice(&view[..]);
                for (pixel, s) in framebuffer.pixels_mut().iter_mut().zip(back) {
                    pixel.sum = Spectrum { r: s.sum[0], g: s.sum[1], b: s.sum[2] };
                    pixel.clamped_energy = s.sum[3];
                    pixel.background_sum = Spectrum { r: s.background[0], g: s.background[1], b: s.background[2] };
                    pixel.coverage_sum = s.background[3];
                    pixel.sum_squared_luminance = s.squares_high as f64 - s.squares_low as f64;
                    pixel.samples = s.samples;
                }
            }
            readback.unmap();
        }
    }
}

