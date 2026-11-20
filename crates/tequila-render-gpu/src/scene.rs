//! A scene on the card (`docs/07` R2.1): its triangles in one buffer of each
//! kind, one bottom-level acceleration structure per mesh, one top level for
//! the instances, and the records a kernel reads to get from a hit back to
//! its mesh, its instance and its material.
//!
//! Every record that crosses to the card is `#[repr(C)]`, and `scene.wgsl`
//! declares its twin; a test reads the WGSL through naga and holds the two
//! layouts to each other member by member (R2.2).

use bytemuck::{Pod, Zeroable};
use tequila_render_core::scene::SceneSnapshot;
use wgpu::util::DeviceExt;

/// Where one mesh lives in the shared buffers.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuMesh {
    /// The mesh's first index in the index buffer.
    pub first_index: u32,
    /// The mesh's first vertex in the position (and normal) buffers; its
    /// indices count from here.
    pub base_vertex: u32,
    /// 1 if the mesh has vertex normals, which are then at `base_vertex` in
    /// the normal buffer; 0 for faceted geometry.
    pub has_normals: u32,
    pub vertex_count: u32,
}

/// One placed instance, at its slot in the top-level structure (the slot is
/// the instance's custom data, which a hit hands back).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuInstance {
    /// Row-major 3 x 4: the part's place, as `Transform::matrix`.
    pub object_to_world: [[f32; 4]; 3],
    /// And its inverse, as `Transform::inverse`, for normals.
    pub world_to_object: [[f32; 4]; 3],
    pub mesh: u32,
    pub material: u32,
    /// The instance's index in the snapshot.
    pub instance: u32,
    pub _pad: u32,
}

/// A snapshot uploaded.
pub struct GpuScene {
    pub positions: wgpu::Buffer,
    pub normals: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub meshes: wgpu::Buffer,
    pub instances: wgpu::Buffer,
    pub blases: Vec<Option<wgpu::Blas>>,
    pub tlas: wgpu::Tlas,
    /// What the records say, kept for tests and for the next upload.
    pub mesh_records: Vec<GpuMesh>,
    pub instance_records: Vec<GpuInstance>,
}

fn rows(m: &tequila_core::math::Mat4) -> [[f32; 4]; 3] {
    [m.m[0], m.m[1], m.m[2]]
}

impl GpuScene {
    /// Upload `scene`'s triangle meshes and the instances of them. A mesh of
    /// fibres has no triangles and is not uploaded (fibres are R3, tier C).
    pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, scene: &SceneSnapshot) -> GpuScene {
        let mut positions: Vec<f32> = Vec::new();
        let mut normals: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut mesh_records = Vec::with_capacity(scene.meshes.len());
        for mesh in &scene.meshes {
            let has_normals = mesh.normals.len() == mesh.positions.len() && !mesh.normals.is_empty();
            mesh_records.push(GpuMesh {
                first_index: indices.len() as u32,
                base_vertex: (positions.len() / 3) as u32,
                has_normals: has_normals as u32,
                vertex_count: mesh.positions.len() as u32,
            });
            for p in &mesh.positions {
                positions.extend_from_slice(&[p.x, p.y, p.z]);
            }
            if has_normals {
                for n in &mesh.normals {
                    normals.extend_from_slice(&[n.x, n.y, n.z]);
                }
            } else {
                // Kept aligned with the positions, so one base serves both.
                normals.resize(positions.len(), 0.0);
            }
            indices.extend_from_slice(&mesh.indices);
        }
        // A buffer may not be empty.
        for buffer in [&mut positions, &mut normals] {
            if buffer.is_empty() {
                buffer.extend_from_slice(&[0.0; 3]);
            }
        }
        if indices.is_empty() {
            indices.extend_from_slice(&[0; 3]);
        }

        let traced: Vec<usize> = scene
            .instances
            .iter()
            .enumerate()
            .filter(|(_, i)| !scene.meshes[i.mesh.index()].indices.is_empty())
            .map(|(n, _)| n)
            .collect();
        let mut instance_records: Vec<GpuInstance> = traced
            .iter()
            .map(|&n| {
                let inst = &scene.instances[n];
                GpuInstance {
                    object_to_world: rows(&inst.transform.matrix),
                    world_to_object: rows(&inst.transform.inverse),
                    mesh: inst.mesh.index() as u32,
                    material: inst.material.index() as u32,
                    instance: n as u32,
                    _pad: 0,
                }
            })
            .collect();
        if instance_records.is_empty() {
            instance_records.push(GpuInstance::default());
        }

        let input = wgpu::BufferUsages::BLAS_INPUT | wgpu::BufferUsages::STORAGE;
        let init = |label: &str, contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage })
        };
        let positions_buffer = init("positions", bytemuck::cast_slice(&positions), input);
        let normals_buffer = init("normals", bytemuck::cast_slice(&normals), wgpu::BufferUsages::STORAGE);
        let indices_buffer = init("indices", bytemuck::cast_slice(&indices), input);
        let meshes_buffer = init("meshes", bytemuck::cast_slice(&mesh_records), wgpu::BufferUsages::STORAGE);
        let instances_buffer = init("instances", bytemuck::cast_slice(&instance_records), wgpu::BufferUsages::STORAGE);

        let sizes: Vec<wgpu::BlasTriangleGeometrySizeDescriptor> = scene
            .meshes
            .iter()
            .map(|mesh| wgpu::BlasTriangleGeometrySizeDescriptor {
                vertex_format: wgpu::VertexFormat::Float32x3,
                vertex_count: mesh.positions.len() as u32,
                index_format: Some(wgpu::IndexFormat::Uint32),
                index_count: Some(mesh.indices.len() as u32),
                flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
            })
            .collect();
        let blases: Vec<Option<wgpu::Blas>> = scene
            .meshes
            .iter()
            .zip(&sizes)
            .map(|(mesh, size)| {
                (!mesh.indices.is_empty()).then(|| {
                    device.create_blas(
                        &wgpu::CreateBlasDescriptor {
                            label: None,
                            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                        },
                        wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![size.clone()] },
                    )
                })
            })
            .collect();
        let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("scene"),
            max_instances: traced.len().max(1) as u32,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        });
        for (slot, record) in instance_records.iter().enumerate().take(traced.len()) {
            let r = record.object_to_world;
            let transform = [
                r[0][0], r[0][1], r[0][2], r[0][3], r[1][0], r[1][1], r[1][2], r[1][3], r[2][0], r[2][1], r[2][2], r[2][3],
            ];
            let blas = blases[record.mesh as usize].as_ref().expect("a traced instance's mesh has triangles");
            tlas[slot] = Some(wgpu::TlasInstance::new(blas, transform, slot as u32, 0xff));
        }
        {
            let entries: Vec<wgpu::BlasBuildEntry> = blases
                .iter()
                .enumerate()
                .filter_map(|(i, b)| b.as_ref().map(|b| (i, b)))
                .map(|(i, blas)| wgpu::BlasBuildEntry {
                    blas,
                    geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                        size: &sizes[i],
                        vertex_buffer: &positions_buffer,
                        first_vertex: mesh_records[i].base_vertex,
                        vertex_stride: 12,
                        index_buffer: Some(&indices_buffer),
                        first_index: Some(mesh_records[i].first_index),
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    }]),
                })
                .collect();
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("build") });
            encoder.build_acceleration_structures(entries.iter(), std::iter::once(&tlas));
            queue.submit(Some(encoder.finish()));
        }
        GpuScene {
            positions: positions_buffer,
            normals: normals_buffer,
            indices: indices_buffer,
            meshes: meshes_buffer,
            instances: instances_buffer,
            blases,
            tlas,
            mesh_records,
            instance_records,
        }
    }
}
