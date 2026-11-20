//! The read-only tables every kernel shares, bound at `@group(1)`.
//!
//! A kernel's own data is `@group(0)`; the tables are the same for every
//! kernel and every frame, uploaded once per device. Today: the Sobol′
//! sampler's byte tables (`tequila_core::sobol::byte_tables`), the very
//! numbers the processor's sampler reads, so the two draw the same samples.

use wgpu::util::DeviceExt;

/// `@group(1) @binding(0)`: `[dimension][byte][value]`, flattened.
pub const SOBOL_BINDING: u32 = 0;

/// The tables, on one device.
pub struct Tables {
    pub sobol: wgpu::Buffer,
    pub layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
}

impl Tables {
    pub fn new(device: &wgpu::Device) -> Tables {
        let flat: Vec<u32> = tequila_core::sobol::byte_tables()
            .iter()
            .flat_map(|dimension| dimension.iter().flat_map(|byte| byte.iter().copied()))
            .collect();
        let sobol = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sobol tables"),
            contents: bytemuck::cast_slice(&flat),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tables"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: SOBOL_BINDING,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tables"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: SOBOL_BINDING, resource: sobol.as_entire_binding() }],
        });
        Tables { sobol, layout, bind_group }
    }
}
