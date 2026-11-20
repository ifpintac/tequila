//! Running a piece of WGSL over a buffer of inputs (`docs/07` R2.4).
//!
//! Every port to the card is tested the same way: random inputs from the
//! ranges the processor's own tests use, the Rust function as the oracle,
//! and [`Harness::run_wgsl`] wrapping the WGSL in a compute shader over two
//! buffers. The body reads item `k`'s inputs with `inp(k)` (a `u32`) or
//! `inf(k)` (an `f32`), and writes its outputs with `out(k, v)` and
//! `outf(k, v)`; the modules it names are included in front of it.

use crate::device::{self, GpuDevice, Needs};
use crate::tables::Tables;
use wgpu::util::DeviceExt;

/// A device and the shared tables, for tests.
pub struct Harness {
    pub gpu: GpuDevice,
    pub tables: Tables,
}

const WORKGROUP: u32 = 64;

impl Harness {
    /// A harness on the best card that can compute, or `None` (a skip) when
    /// there is none and `TEQUILA_REQUIRE_GPU_TESTS` is not set.
    pub fn for_test() -> Option<Harness> {
        Harness::with(Needs::Compute)
    }

    pub fn with(needs: Needs) -> Option<Harness> {
        let gpu = device::for_test(needs)?;
        let tables = Tables::new(&gpu.device);
        Some(Harness { gpu, tables })
    }

    /// Run `body` once per item over `inputs`, `in_words` words an item, and
    /// return `out_words` words an item. `modules` are included in front.
    pub fn run_wgsl(&self, modules: &[&str], body: &str, inputs: &[u32], in_words: usize, out_words: usize) -> Vec<u32> {
        self.run_wgsl_with(modules, body, inputs, in_words, out_words, &[])
    }

    /// [`Harness::run_wgsl`] for modules that read buffers of their own at
    /// `@group(0)` (`lamps.wgsl`'s lamps): each of `buffers` bound, read
    /// only, at its binding.
    pub fn run_wgsl_with(
        &self,
        modules: &[&str],
        body: &str,
        inputs: &[u32],
        in_words: usize,
        out_words: usize,
        buffers: &[(u32, &[u32])],
    ) -> Vec<u32> {
        assert!(in_words > 0 && inputs.len() % in_words == 0, "inputs are whole items");
        let count = (inputs.len() / in_words) as u32;
        let groups = count.div_ceil(WORKGROUP);
        let max = self.gpu.device.limits().max_compute_workgroups_per_dimension;
        let (gx, gy) = if groups <= max { (groups, 1) } else { (max, groups.div_ceil(max)) };
        let includes: String = modules.iter().map(|m| format!("//#include {m}\n")).collect();
        let kernel = format!(
            "{includes}
@group(0) @binding(0) var<storage, read> harness_input: array<u32>;
@group(0) @binding(1) var<storage, read_write> harness_output: array<u32>;
var<private> harness_item: u32;
fn inp(k: u32) -> u32 {{ return harness_input[harness_item * {in_words}u + k]; }}
fn inf(k: u32) -> f32 {{ return bitcast<f32>(harness_input[harness_item * {in_words}u + k]); }}
fn out(k: u32, v: u32) {{ harness_output[harness_item * {out_words}u + k] = v; }}
fn outf(k: u32, v: f32) {{ harness_output[harness_item * {out_words}u + k] = bitcast<u32>(v); }}
@compute @workgroup_size({WORKGROUP})
fn harness_main(@builtin(global_invocation_id) id: vec3<u32>) {{
    harness_item = id.x + id.y * {gx}u * {WORKGROUP}u;
    if (harness_item >= {count}u) {{ return; }}
    {body}
}}
"
        );
        self.run_kernel_with(&crate::shader::with_includes(&kernel), "harness_main", inputs, count as usize * out_words, (gx, gy), buffers)
    }

    /// Run a whole kernel: `@group(0)` binding 0 the inputs, binding 1 the
    /// outputs, `@group(1)` the shared tables (when the kernel uses them).
    pub fn run_kernel(&self, source: &str, entry: &str, inputs: &[u32], outputs: usize, groups: (u32, u32)) -> Vec<u32> {
        self.run_kernel_with(source, entry, inputs, outputs, groups, &[])
    }

    /// [`Harness::run_kernel`] with `buffers` bound, read only, at their
    /// bindings in `@group(0)` as well.
    pub fn run_kernel_with(
        &self,
        source: &str,
        entry: &str,
        inputs: &[u32],
        outputs: usize,
        (gx, gy): (u32, u32),
        buffers: &[(u32, &[u32])],
    ) -> Vec<u32> {
        let device = &self.gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(entry),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let uses_tables = source.contains("@group(1)");
        let mut entries = vec![storage(0, true), storage(1, false)];
        entries.extend(buffers.iter().map(|(binding, _)| storage(*binding, true)));
        let own = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("harness"), entries: &entries });
        let mut layouts: Vec<&wgpu::BindGroupLayout> = vec![&own];
        if uses_tables {
            layouts.push(&self.tables.layout);
        }
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("harness"),
            bind_group_layouts: &layouts.iter().map(|l| Some(*l)).collect::<Vec<_>>(),
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(&layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });
        // A buffer may not be empty.
        let padded: Vec<u32> = if inputs.is_empty() { vec![0] } else { inputs.to_vec() };
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("harness input"),
            contents: bytemuck::cast_slice(&padded),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let bytes = (outputs.max(1) * 4) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("harness output"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("harness readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let extra: Vec<wgpu::Buffer> = buffers
            .iter()
            .map(|(_, words)| {
                let padded: Vec<u32> = if words.is_empty() { vec![0] } else { words.to_vec() };
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("harness buffer"),
                    contents: bytemuck::cast_slice(&padded),
                    usage: wgpu::BufferUsages::STORAGE,
                })
            })
            .collect();
        let mut bound = vec![
            wgpu::BindGroupEntry { binding: 0, resource: input.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
        ];
        bound.extend(buffers.iter().zip(&extra).map(|((binding, _), buffer)| wgpu::BindGroupEntry { binding: *binding, resource: buffer.as_entire_binding() }));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("harness"), layout: &own, entries: &bound });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("harness") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("harness"), timestamp_writes: None });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            if uses_tables {
                pass.set_bind_group(1, &self.tables.bind_group, &[]);
            }
            pass.dispatch_workgroups(gx, gy, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        self.gpu.queue.submit(Some(encoder.finish()));
        readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("the harness output maps"));
        self.gpu.wait();
        let view = readback.slice(..).get_mapped_range().expect("the harness output is mapped");
        let mut words: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&view[..]).to_vec();
        words.truncate(outputs);
        words
    }
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
