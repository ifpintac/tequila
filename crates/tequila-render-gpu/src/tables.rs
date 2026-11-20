//! The read-only tables every kernel shares, bound at `@group(1)`.
//!
//! A kernel's own data is `@group(0)`; the tables are the same for every
//! kernel and every frame, uploaded once per device: the Sobol′ sampler's
//! byte tables (`tequila_core::sobol::byte_tables`) and the GGX energy
//! tables (`tequila_materials::energy::GgxEnergyTable::raw`), the very
//! numbers the processor reads, so the two draw the same samples and
//! compensate their lobes alike.

use wgpu::util::DeviceExt;

/// `@group(1) @binding(0)`: `[dimension][byte][value]`, flattened.
pub const SOBOL_BINDING: u32 = 0;
/// `@group(1) @binding(1)`: the energy tables end to end, in
/// `ENERGY_TABLE_LAYOUT`'s order.
pub const ENERGY_BINDING: u32 = 1;

/// Where `energy.wgsl` expects each energy table to start.
pub const ENERGY_OFFSETS: [usize; 8] = [0, 1024, 2048, 2080, 2112, 2144, 18528, 19040];

/// The tables, on one device.
pub struct Tables {
    pub sobol: wgpu::Buffer,
    pub energy: wgpu::Buffer,
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
        let mut energy_values: Vec<f32> = Vec::new();
        let raw = tequila_materials::energy::table().raw();
        for (i, ((name, length), values)) in tequila_materials::energy::ENERGY_TABLE_LAYOUT.iter().zip(raw).enumerate() {
            assert_eq!(energy_values.len(), ENERGY_OFFSETS[i], "energy.wgsl expects {name} elsewhere");
            assert_eq!(values.len(), *length, "{name} is not the length its layout says");
            energy_values.extend_from_slice(values);
        }
        let energy = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("energy tables"),
            contents: bytemuck::cast_slice(&energy_values),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let read_only = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tables"),
            entries: &[read_only(SOBOL_BINDING), read_only(ENERGY_BINDING)],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tables"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: SOBOL_BINDING, resource: sobol.as_entire_binding() },
                wgpu::BindGroupEntry { binding: ENERGY_BINDING, resource: energy.as_entire_binding() },
            ],
        });
        Tables { sobol, energy, layout, bind_group }
    }
}
