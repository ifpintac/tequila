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
    /// The mesh's first UV in the UV buffer, [`NO_DATA`] for a mesh with
    /// none: a hit's UV is then its barycentrics, as Embree's resolve has it.
    pub uv_base: u32,
    /// Its first tangent in the tangent buffer, [`NO_DATA`] for none.
    pub tangent_base: u32,
    /// Its first vertex in the attribute buffer, two vec4s a vertex - its
    /// curvature, occlusion and closeness, then its colour - [`NO_DATA`]
    /// for a mesh with none or a scene no program of which reads them.
    pub attribute_base: u32,
    /// Which it has: [`HAS_CURVATURE`], [`HAS_OCCLUSION`],
    /// [`HAS_CLOSENESS`], [`HAS_COLORS`].
    pub attribute_flags: u32,
    /// Its bounds in its own space (`Mesh::bounds`), which the Generated
    /// texture coordinate is normalized to.
    pub bounds_min: [f32; 4],
    pub bounds_max: [f32; 4],
}

/// [`GpuMesh::uv_base`] or [`GpuMesh::tangent_base`] of a mesh without.
pub const NO_DATA: u32 = u32::MAX;
/// [`GpuMesh::attribute_flags`]: what of `Mesh`'s per-vertex measures it
/// has, each a list as long as its vertices (one that is not is none).
pub const HAS_CURVATURE: u32 = 1;
pub const HAS_OCCLUSION: u32 = 2;
pub const HAS_CLOSENESS: u32 = 4;
pub const HAS_COLORS: u32 = 8;

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
    /// Its shadow catcher's record in the catcher buffer, [`NO_CATCHER`]
    /// for an ordinary surface.
    pub catcher: u32,
    /// The lamp it stands in for (`Instance::light`), by its place among
    /// the scene's lamps; [`NO_LIGHT`] for any other instance.
    pub light: u32,
    /// [`INSTANCE_STAND_IN`] for a lamp's stand-in (`Instance::light_proxy`),
    /// [`INSTANCE_CASTS_SHADOWS`] for a part that casts shadows.
    pub flags: u32,
    /// The solid a medium is keyed by: what a path is inside of, what the
    /// probe counts crossings of, what a shadow ray's walk pairs its way
    /// into a volume with. The instance here (integrator.rs keys media by
    /// instance); a solid split into parts would share one.
    pub solid: u32,
    pub _pad: u32,
    /// What Object Info reads: the colour set on the part (the fourth float
    /// unused).
    pub object_color: [f32; 4],
    /// `Transform::inv_length_scale`, and the scene units per unit of the
    /// part's own space (its inverse, as the shading point holds it).
    pub inv_length_scale: f32,
    pub scale: f32,
    /// `SceneSnapshot::object_random` of the instance, and its number.
    pub object_random: f32,
    pub object_number: f32,
}

/// [`GpuInstance::catcher`] of an instance that is no shadow catcher.
pub const NO_CATCHER: u32 = u32::MAX;
/// [`GpuInstance::light`] of an instance that stands in for no lamp.
pub const NO_LIGHT: u32 = u32::MAX;
/// [`GpuInstance::flags`]: a lamp's visible stand-in.
pub const INSTANCE_STAND_IN: u32 = 1;
/// [`GpuInstance::flags`]: the part casts shadows (`Instance::casts_shadows`):
/// a shadow ray's walk through glass passes one that does not.
pub const INSTANCE_CASTS_SHADOWS: u32 = 2;

/// The top-level structure's instance masks, as Embree's geometry masks are
/// (`tequila-render-cpu`'s `embree.rs`): every instance a surface, and the
/// ones that cast shadows also in the mask shadow rays look for. A lamp's
/// stand-in and a part that casts no shadow are passed by every shadow ray.
pub const MASK_SURFACE: u8 = 1;
pub const MASK_SHADOW: u8 = 2;

/// A shadow catcher (`tequila_render_core::scene::ShadowCatcher`) as the card
/// reads it: the floor the environment's ground puts in, or a part made one.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct GpuCatcher {
    /// The shadow's tint; the fourth float unused.
    pub shadow_color: [f32; 4],
    /// The glow the product sees of the floor (`ShadowCatcher::bounce`), and
    /// in the fourth float 1 when there is one.
    pub bounce: [f32; 4],
    pub reflections: f32,
    pub reflection_roughness: f32,
    /// The height, or with [`FADE_BY_DISTANCE`] the distance, the reflection
    /// fades out by; read only with a fade.
    pub reflection_fade: f32,
    /// [`FADE_NONE`], [`FADE_BY_HEIGHT`] or [`FADE_BY_DISTANCE`].
    pub fade: u32,
    pub reflection_fresnel: u32,
    pub occlusion_only: u32,
    pub occlusion_radius: f32,
    pub shadows: u32,
    pub bounce_falloff: f32,
    pub _pad: [u32; 3],
}

/// [`GpuCatcher::fade`]: a reflection that never fades.
pub const FADE_NONE: u32 = 0;
/// [`GpuCatcher::fade`]: fading with the height of what it shows.
pub const FADE_BY_HEIGHT: u32 = 1;
/// [`GpuCatcher::fade`]: fading with how near the product is to the floor point.
pub const FADE_BY_DISTANCE: u32 = 2;

impl GpuCatcher {
    pub fn from_catcher(c: &tequila_render_core::scene::ShadowCatcher) -> GpuCatcher {
        let s = c.shadow_color;
        let (fade, reflection_fade) = match c.reflection_fade {
            None => (FADE_NONE, 0.0),
            Some(fade) if c.reflection_fade_by_distance => (FADE_BY_DISTANCE, fade),
            Some(fade) => (FADE_BY_HEIGHT, fade),
        };
        GpuCatcher {
            shadow_color: [s.r, s.g, s.b, 0.0],
            bounce: match c.bounce {
                Some(glow) => [glow.r, glow.g, glow.b, 1.0],
                None => [0.0; 4],
            },
            reflections: c.reflections,
            reflection_roughness: c.reflection_roughness,
            reflection_fade,
            fade,
            reflection_fresnel: c.reflection_fresnel as u32,
            occlusion_only: c.occlusion_only as u32,
            occlusion_radius: c.occlusion_radius,
            shadows: c.shadows as u32,
            bounce_falloff: c.bounce_falloff,
            _pad: [0; 3],
        }
    }
}

/// A snapshot uploaded.
pub struct GpuScene {
    pub positions: wgpu::Buffer,
    pub normals: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub meshes: wgpu::Buffer,
    pub instances: wgpu::Buffer,
    /// The shadow catchers' records, by [`GpuInstance::catcher`].
    pub catchers: wgpu::Buffer,
    /// Every mesh's UVs, two floats a vertex, from its `uv_base`.
    pub uvs: wgpu::Buffer,
    /// Every mesh's tangents, four floats a vertex (the handedness in the
    /// fourth), from its `tangent_base`.
    pub tangents: wgpu::Buffer,
    /// Every mesh's per-vertex measures, two vec4s a vertex, from its
    /// `attribute_base`.
    pub attributes: wgpu::Buffer,
    pub blases: Vec<Option<wgpu::Blas>>,
    pub tlas: wgpu::Tlas,
    /// What the records say, kept for tests and for the next upload.
    pub mesh_records: Vec<GpuMesh>,
    pub instance_records: Vec<GpuInstance>,
    pub catcher_records: Vec<GpuCatcher>,
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
        let mut uvs: Vec<f32> = Vec::new();
        let mut tangents: Vec<f32> = Vec::new();
        let mut attributes: Vec<f32> = Vec::new();
        // The per-vertex measures only where a program reads them.
        let measures = scene.programs.iter().flatten().any(|p| crate::program::reads_attributes(p));
        let mut mesh_records = Vec::with_capacity(scene.meshes.len());
        for mesh in &scene.meshes {
            let has_normals = mesh.normals.len() == mesh.positions.len() && !mesh.normals.is_empty();
            // As `uv_derivatives` and Embree's resolve read them: a UV, or a
            // tangent, for every vertex or none at all.
            let has_uvs = !mesh.uvs.is_empty() && mesh.uvs.len() == mesh.positions.len();
            let has_tangents = !mesh.tangents.is_empty() && mesh.tangents.len() == mesh.positions.len();
            let bounds = mesh.bounds();
            let has = |len: usize, flag: u32| if measures && len == mesh.positions.len() && len > 0 { flag } else { 0 };
            let attribute_flags = has(mesh.curvature.len(), HAS_CURVATURE)
                | has(mesh.occlusion.len(), HAS_OCCLUSION)
                | has(mesh.closeness.len(), HAS_CLOSENESS)
                | has(mesh.colors.len(), HAS_COLORS);
            mesh_records.push(GpuMesh {
                first_index: indices.len() as u32,
                base_vertex: (positions.len() / 3) as u32,
                has_normals: has_normals as u32,
                vertex_count: mesh.positions.len() as u32,
                uv_base: if has_uvs { (uvs.len() / 2) as u32 } else { NO_DATA },
                tangent_base: if has_tangents { (tangents.len() / 4) as u32 } else { NO_DATA },
                attribute_base: if attribute_flags != 0 { (attributes.len() / 8) as u32 } else { NO_DATA },
                attribute_flags,
                bounds_min: [bounds.min.x, bounds.min.y, bounds.min.z, 0.0],
                bounds_max: [bounds.max.x, bounds.max.y, bounds.max.z, 0.0],
            });
            if has_uvs {
                for t in &mesh.uvs {
                    uvs.extend_from_slice(&[t.x, t.y]);
                }
            }
            if has_tangents {
                for t in &mesh.tangents {
                    tangents.extend_from_slice(&[t.x, t.y, t.z, t.w]);
                }
            }
            if attribute_flags != 0 {
                let at = |values: &Vec<f32>, flag: u32, i: usize| if attribute_flags & flag != 0 { values[i] } else { 0.0 };
                for i in 0..mesh.positions.len() {
                    let c = if attribute_flags & HAS_COLORS != 0 { mesh.colors[i] } else { tequila_core::math::Vec4 { x: 1.0, y: 1.0, z: 1.0, w: 1.0 } };
                    attributes.extend_from_slice(&[
                        at(&mesh.curvature, HAS_CURVATURE, i),
                        at(&mesh.occlusion, HAS_OCCLUSION, i),
                        at(&mesh.closeness, HAS_CLOSENESS, i),
                        0.0,
                        c.x,
                        c.y,
                        c.z,
                        c.w,
                    ]);
                }
            }
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
        for buffer in [&mut positions, &mut normals, &mut uvs, &mut tangents] {
            if buffer.is_empty() {
                buffer.extend_from_slice(&[0.0; 3]);
            }
        }
        if attributes.is_empty() {
            attributes.extend_from_slice(&[0.0; 8]);
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
        let mut catcher_records: Vec<GpuCatcher> = Vec::new();
        let mut instance_records: Vec<GpuInstance> = traced
            .iter()
            .map(|&n| {
                let inst = &scene.instances[n];
                let catcher = match &inst.shadow_catcher {
                    Some(c) => {
                        catcher_records.push(GpuCatcher::from_catcher(c));
                        catcher_records.len() as u32 - 1
                    }
                    None => NO_CATCHER,
                };
                GpuInstance {
                    object_to_world: rows(&inst.transform.matrix),
                    world_to_object: rows(&inst.transform.inverse),
                    mesh: inst.mesh.index() as u32,
                    material: inst.material.index() as u32,
                    instance: n as u32,
                    catcher,
                    light: inst.light.unwrap_or(NO_LIGHT),
                    flags: if inst.light_proxy { INSTANCE_STAND_IN } else { 0 } | if inst.casts_shadows { INSTANCE_CASTS_SHADOWS } else { 0 },
                    solid: n as u32,
                    _pad: 0,
                    object_color: [inst.colour.r, inst.colour.g, inst.colour.b, 0.0],
                    inv_length_scale: inst.transform.inv_length_scale(),
                    scale: 1.0 / inst.transform.inv_length_scale(),
                    object_random: SceneSnapshot::object_random(n),
                    object_number: inst.number,
                }
            })
            .collect();
        if instance_records.is_empty() {
            instance_records.push(GpuInstance { catcher: NO_CATCHER, light: NO_LIGHT, ..GpuInstance::default() });
        }
        let mut catcher_buffer_records = catcher_records.clone();
        if catcher_buffer_records.is_empty() {
            catcher_buffer_records.push(GpuCatcher::default());
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
        let catchers_buffer = init("catchers", bytemuck::cast_slice(&catcher_buffer_records), wgpu::BufferUsages::STORAGE);
        let uvs_buffer = init("uvs", bytemuck::cast_slice(&uvs), wgpu::BufferUsages::STORAGE);
        let tangents_buffer = init("tangents", bytemuck::cast_slice(&tangents), wgpu::BufferUsages::STORAGE);
        let attributes_buffer = init("attributes", bytemuck::cast_slice(&attributes), wgpu::BufferUsages::STORAGE);

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
            let shadow = if scene.instances[traced[slot]].casts_shadows { MASK_SHADOW } else { 0 };
            tlas[slot] = Some(wgpu::TlasInstance::new(blas, transform, slot as u32, MASK_SURFACE | shadow));
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
            catchers: catchers_buffer,
            uvs: uvs_buffer,
            tangents: tangents_buffer,
            attributes: attributes_buffer,
            blases,
            tlas,
            mesh_records,
            instance_records,
            catcher_records,
        }
    }
}
