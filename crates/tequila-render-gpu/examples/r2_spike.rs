//! R2.0 spike (`docs/07-OWN-GPU-RENDERER.md`): a project's geometry through
//! wgpu ray queries on the graphics card and through Embree on the processor,
//! doing the same work - a camera ray and eight ambient-occlusion rays per
//! pixel - and the numbers R2.0 asks for: rays per second on both, the cold
//! pipeline compile, the acceleration-structure build, card memory.
//!
//! cargo run --release -p tequila-render-gpu --example r2_spike -- [project] [width height] [out-dir]
//!
//! A throwaway branch: none of this is meant to be merged.

use std::path::{Path, PathBuf};
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use rayon::prelude::*;
use tequila_core::math::{vec3, Ray, Vec3};
use tequila_render_core::intersect::Intersector;
use tequila_render_cpu::{Device as EmbreeDevice, EmbreeIntersector};
use wgpu::util::DeviceExt;

const AO_RAYS: u32 = 8;
const GPU_ITERATIONS: u32 = 16;
const CPU_ITERATIONS: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    origin: [f32; 3],
    width: u32,
    right: [f32; 3],
    height: u32,
    up: [f32; 3],
    ao_rays: u32,
    forward: [f32; 3],
    radius: f32,
    seed: u32,
    _pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeshRange {
    first_index: u32,
    base_vertex: u32,
    _pad: [u32; 2],
}

fn hash(x: u32) -> u32 {
    let mut v = x;
    v ^= v >> 16;
    v = v.wrapping_mul(0x7feb352d);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846ca68b);
    v ^= v >> 16;
    v
}

fn unit(x: u32) -> f32 {
    (x >> 8) as f32 * (1.0 / 16_777_216.0)
}

fn arr(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().unwrap_or_else(|| "assets/samples/Headphones.tequila".into());
    let width: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(960);
    let height: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(540);
    let out = PathBuf::from(args.get(4).cloned().unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&out).ok();

    // --- The scene, as the renderers are handed it ------------------------
    let t = Instant::now();
    let loaded = tequila_format::load(Path::new(&path)).expect("the project loads");
    let document = loaded.document;
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (scene, _) =
        tequila_scene::snapshot::snapshot(&document, environment, &mut tequila_scene::snapshot::MeshCache::new());
    let camera = document.cameras.active().camera;
    println!(
        "scene: {} meshes, {} instances, {} triangles; loaded in {:.2} s",
        scene.meshes.len(),
        scene.instances.len(),
        scene.triangle_count(),
        t.elapsed().as_secs_f64()
    );

    let aspect = width as f32 / height as f32;
    let m = camera.camera_to_world;
    let half_h = (camera.vertical_fov(aspect) * 0.5).tan();
    let half_w = half_h * aspect;
    let origin = m.transform_point(Vec3::ZERO);
    let right = m.transform_vector(vec3(half_w, 0.0, 0.0));
    let up = m.transform_vector(vec3(0.0, half_h, 0.0));
    let forward = m.transform_vector(vec3(0.0, 0.0, -1.0));
    let bounds = scene.bounds();
    let radius = (bounds.max - bounds.min).length() * 0.05;
    println!("AO radius {radius:.4} (5% of the scene's diagonal)");

    // --- Geometry, one buffer of each for every mesh -----------------------
    let mut positions: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut ranges: Vec<MeshRange> = Vec::new();
    for mesh in &scene.meshes {
        ranges.push(MeshRange { first_index: indices.len() as u32, base_vertex: (positions.len() / 3) as u32, _pad: [0; 2] });
        for p in &mesh.positions {
            positions.extend_from_slice(&[p.x, p.y, p.z]);
        }
        indices.extend_from_slice(&mesh.indices);
    }
    let traced: Vec<usize> = scene
        .instances
        .iter()
        .enumerate()
        .filter(|(_, i)| !scene.meshes[i.mesh.index()].indices.is_empty())
        .map(|(n, _)| n)
        .collect();

    // --- The card -----------------------------------------------------------
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("a Vulkan adapter");
    let info = adapter.get_info();
    println!("adapter: {} ({:?}, {} {})", info.name, info.backend, info.driver, info.driver_info);
    let features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;
    assert!(adapter.features().contains(features), "{} has no ray queries", info.name);
    let t = Instant::now();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("r2 spike"),
        required_features: features,
        required_limits: adapter.limits(),
        // SAFETY: a spike; ray queries are an experimental feature in wgpu 30.
        experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }))
    .expect("a device with ray queries");
    println!("device in {:.3} s", t.elapsed().as_secs_f64());

    let as_input = wgpu::BufferUsages::BLAS_INPUT | wgpu::BufferUsages::STORAGE;
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("positions"),
        contents: bytemuck::cast_slice(&positions),
        usage: as_input,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("indices"),
        contents: bytemuck::cast_slice(&indices),
        usage: as_input,
    });
    let mesh_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("meshes"),
        contents: bytemuck::cast_slice(&ranges),
        usage: wgpu::BufferUsages::STORAGE,
    });

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
    let used: Vec<bool> = scene.meshes.iter().map(|m| !m.indices.is_empty()).collect();
    let blases: Vec<Option<wgpu::Blas>> = sizes
        .iter()
        .zip(&used)
        .map(|(size, &used)| {
            used.then(|| {
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
        max_instances: traced.len() as u32,
        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    });
    for (slot, &n) in traced.iter().enumerate() {
        let inst = &scene.instances[n];
        let mesh = inst.mesh.index();
        let r = inst.transform.matrix.m;
        let transform = [
            r[0][0], r[0][1], r[0][2], r[0][3], r[1][0], r[1][1], r[1][2], r[1][3], r[2][0], r[2][1], r[2][2], r[2][3],
        ];
        tlas[slot] = Some(wgpu::TlasInstance::new(blases[mesh].as_ref().unwrap(), transform, mesh as u32, 0xff));
    }

    let t = Instant::now();
    {
        let geometries: Vec<(usize, wgpu::BlasBuildEntry)> = blases
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.as_ref().map(|b| (i, b)))
            .map(|(i, blas)| {
                (
                    i,
                    wgpu::BlasBuildEntry {
                        blas,
                        geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                            size: &sizes[i],
                            vertex_buffer: &vertex_buffer,
                            first_vertex: ranges[i].base_vertex,
                            vertex_stride: 12,
                            index_buffer: Some(&index_buffer),
                            first_index: Some(ranges[i].first_index),
                            transform_buffer: None,
                            transform_buffer_offset: None,
                        }]),
                    },
                )
            })
            .collect();
        let entries: Vec<&wgpu::BlasBuildEntry> = geometries.iter().map(|(_, e)| e).collect();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("build") });
        encoder.build_acceleration_structures(entries.into_iter(), std::iter::once(&tlas));
        queue.submit(Some(encoder.finish()));
    }
    device.poll(wgpu::PollType::wait_indefinitely()).expect("the build finishes");
    let build_time = t.elapsed().as_secs_f64();
    println!("acceleration structures: {} BLAS, {} instances, built in {:.3} s", used.iter().filter(|u| **u).count(), traced.len(), build_time);

    let t = Instant::now();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("r2 spike"),
        source: wgpu::ShaderSource::Wgsl(include_str!("r2_spike.wgsl").into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ao"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    device.poll(wgpu::PollType::wait_indefinitely()).ok();
    let compile_time = t.elapsed().as_secs_f64();
    println!("pipeline compiled in {compile_time:.3} s (cold only on a first run: the driver caches it)");

    let pixels = (width * height) as usize;
    let params = |seed: u32| Params {
        origin: arr(origin),
        width,
        right: arr(right),
        height,
        up: arr(up),
        ao_rays: AO_RAYS,
        forward: arr(forward),
        radius,
        seed,
        _pad: [0; 3],
    };
    let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("params"),
        contents: bytemuck::bytes_of(&params(0)),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let result_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("result"),
        size: (pixels * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (pixels * 4) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ao"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::AccelerationStructure(&tlas) },
            wgpu::BindGroupEntry { binding: 1, resource: params_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: vertex_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: index_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: mesh_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: result_buffer.as_entire_binding() },
        ],
    });
    let dispatch = |seed: u32| {
        queue.write_buffer(&params_buffer, 0, bytemuck::bytes_of(&params(seed)));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ao") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("ao"), timestamp_writes: None });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        queue.submit(Some(encoder.finish()));
    };

    // Warm up once, then time.
    let t = Instant::now();
    dispatch(0);
    device.poll(wgpu::PollType::wait_indefinitely()).expect("the first frame finishes");
    println!("first frame (including any late pipeline work) {:.3} s", t.elapsed().as_secs_f64());
    let t = Instant::now();
    for seed in 1..=GPU_ITERATIONS {
        dispatch(seed);
    }
    device.poll(wgpu::PollType::wait_indefinitely()).expect("the frames finish");
    let gpu_time = t.elapsed().as_secs_f64();

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("readback") });
    encoder.copy_buffer_to_buffer(&result_buffer, 0, &readback, 0, (pixels * 4) as u64);
    queue.submit(Some(encoder.finish()));
    readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("maps"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("the readback finishes");
    let gpu: Vec<f32> = {
        let view = readback.slice(..).get_mapped_range().expect("the readback is mapped");
        bytemuck::cast_slice::<u8, f32>(&view[..]).to_vec()
    };
    let gpu_hits = gpu.iter().filter(|v| **v >= 0.0).count();
    let gpu_rays = GPU_ITERATIONS as f64 * (pixels + gpu_hits * AO_RAYS as usize) as f64;
    println!(
        "card: {GPU_ITERATIONS} frames in {gpu_time:.3} s, {:.1} M rays/s ({:.1}% of pixels hit)",
        gpu_rays / gpu_time / 1e6,
        100.0 * gpu_hits as f64 / pixels as f64
    );
    if let Some(report) = device.generate_allocator_report() {
        println!(
            "card memory: {:.1} MiB allocated, {:.1} MiB reserved",
            report.total_allocated_bytes as f64 / 1048576.0,
            report.total_reserved_bytes as f64 / 1048576.0
        );
    }

    // --- The processor, the same rays ---------------------------------------
    let embree = EmbreeDevice::shared().expect("an Embree device");
    let t = Instant::now();
    let intersector = EmbreeIntersector::build(&embree, scene.clone()).expect("Embree builds");
    println!("Embree: built in {:.3} s", t.elapsed().as_secs_f64());
    let trace = |pixel: u32, seed: u32| -> f32 {
        let (x, y) = (pixel % width, pixel / width);
        let sx = 2.0 * (x as f32 + 0.5) / width as f32 - 1.0;
        let sy = 1.0 - 2.0 * (y as f32 + 0.5) / height as f32;
        let dir = (forward + right * sx + up * sy).normalize();
        let Some(hit) = intersector.intersect(&Ray::new(origin, dir)) else {
            return -1.0;
        };
        let mut n = hit.geometric_normal;
        if n.dot(dir) > 0.0 {
            n = -n;
        }
        let p = hit.position + n * (1e-4 * radius);
        let s = if n.x.abs() > 0.9 { vec3(0.0, 1.0, 0.0) } else { vec3(1.0, 0.0, 0.0) };
        let t = n.cross(s).normalize();
        let b = n.cross(t);
        let mut open = 0;
        for k in 0..AO_RAYS {
            let h = hash(pixel.wrapping_mul(9781).wrapping_add(k.wrapping_mul(6271)).wrapping_add(seed.wrapping_mul(26699)));
            let (u1, u2) = (unit(h), unit(hash(h ^ 0x9e37_79b9)));
            let r = u1.sqrt();
            let phi = std::f32::consts::TAU * u2;
            let d = (t * (r * phi.cos()) + b * (r * phi.sin()) + n * (1.0 - u1).max(0.0).sqrt()).normalize();
            if !intersector.occluded(p, n, p + d * radius, Vec3::ZERO) {
                open += 1;
            }
        }
        open as f32 / AO_RAYS as f32
    };
    let _ = (0..pixels as u32).into_par_iter().map(|p| trace(p, 0)).count();
    let t = Instant::now();
    let mut cpu = Vec::new();
    for seed in (GPU_ITERATIONS - CPU_ITERATIONS + 1)..=GPU_ITERATIONS {
        cpu = (0..pixels as u32).into_par_iter().map(|p| trace(p, seed)).collect::<Vec<f32>>();
    }
    let cpu_time = t.elapsed().as_secs_f64();
    let cpu_hits = cpu.iter().filter(|v| **v >= 0.0).count();
    let cpu_rays = CPU_ITERATIONS as f64 * (pixels + cpu_hits * AO_RAYS as usize) as f64;
    println!(
        "processor ({} threads): {CPU_ITERATIONS} frames in {cpu_time:.3} s, {:.1} M rays/s",
        rayon::current_num_threads(),
        cpu_rays / cpu_time / 1e6
    );
    println!("card / processor: {:.1}x", (gpu_rays / gpu_time) / (cpu_rays / cpu_time));

    // --- Do they see the same picture? ---------------------------------------
    let both: Vec<(f32, f32)> = gpu.iter().zip(&cpu).filter(|(g, c)| **g >= 0.0 && **c >= 0.0).map(|(g, c)| (*g, *c)).collect();
    let disagree = gpu.iter().zip(&cpu).filter(|(g, c)| (**g >= 0.0) != (**c >= 0.0)).count();
    let mean = |f: &dyn Fn(&(f32, f32)) -> f32| both.iter().map(f).sum::<f32>() / both.len().max(1) as f32;
    println!(
        "pictures: hit/miss differ at {disagree} pixels ({:.3}%); mean AO card {:.4}, processor {:.4}, mean |difference| {:.4} (8 rays: noise)",
        100.0 * disagree as f64 / pixels as f64,
        mean(&|p| p.0),
        mean(&|p| p.1),
        mean(&|p| (p.0 - p.1).abs())
    );
    for (name, values) in [("card", &gpu), ("processor", &cpu)] {
        let file = std::fs::File::create(out.join(format!("r2-spike-{name}.png"))).expect("writes");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgb);
        let mut writer = encoder.write_header().expect("header");
        let data: Vec<u8> = values
            .iter()
            .flat_map(|v| if *v < 0.0 { [40u8, 44, 52] } else { let g = (v.powf(1.0 / 2.2) * 255.0) as u8; [g, g, g] })
            .collect();
        writer.write_image_data(&data).expect("pixels");
    }
    println!("wrote {}", out.join("r2-spike-card.png").display());
}
