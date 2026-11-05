//! §4.4's fibres through the whole transport loop.
//!
//! The fibre BSDF's own tests hold `sample`, `eval` and `pdf` together at
//! one fibre. What they cannot see is the integrator around it: the frame
//! it is shaded in, the cosine it divides out, the side a path leaves by,
//! the rays that leave a fibre and must not meet it again. In a uniform
//! white environment a lossless object is invisible, fur and all: a white
//! ball grown over with white fibres reads the environment's radiance at
//! every pixel, whatever the fibres do to the paths, and a pixel that reads
//! less is light the transport lost.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::FuzzRequest;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

/// A unit ball in `material`, with the fibres the material asks for, seen
/// whole from in front, in a white environment.
fn furred_ball(material: Material, config: IntegratorConfig) -> Framebuffer {
    let ball = tequila_scene::demo::uv_sphere(1.0, 96, 48);
    let mut meshes = vec![Arc::new(ball.clone())];
    let mut instances = vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))];
    if let Some(request) = FuzzRequest::of_material(&material) {
        let fur = tequila_scene::fuzz::grow(&ball, 1.0, &request).expect("fibres");
        meshes.push(Arc::new(fur));
        instances.push(Instance::new(InstanceId(1), MeshId(1), MaterialId(0)));
    }
    let scene = SceneSnapshot::new(meshes, instances, vec![material], Arc::new(Environment::uniform(Spectrum::ONE)));
    let intersector = EmbreeIntersector::build(&Device::shared().expect("an Embree device"), scene).expect("builds");
    let mut framebuffer = Framebuffer::new(48, 48);
    let context = RenderContext {
        camera: camera(),
        config,
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 256);
    framebuffer
}

/// The mean over the middle of the image, fur seen end on over the ball.
fn centre(image: &Framebuffer) -> Spectrum {
    let mut total = Spectrum::ZERO;
    for y in 18..30 {
        for x in 18..30 {
            total = total + image.pixel(x, y).mean();
        }
    }
    total * (1.0 / 144.0)
}

fn camera() -> tequila_render_core::camera::Camera {
    tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::Y)
}

/// The mean round the outline, fur seen side on against the sky.
fn rim(image: &Framebuffer) -> Spectrum {
    // Where the outline falls: the ball's edge seen from the camera, a
    // radius over at the distance where its tangent rays touch it.
    let edge = camera().project(vec3(0.9682, 0.0, 0.25), 1.0).expect("in view");
    let outline = (edge.x - 0.5).abs() * 48.0;
    let (mut total, mut n) = (Spectrum::ZERO, 0.0);
    for y in 0..48 {
        for x in 0..48 {
            let r = ((x as f32 + 0.5 - 24.0).powi(2) + (y as f32 + 0.5 - 24.0).powi(2)).sqrt();
            if (outline - 1.5..outline + 2.0).contains(&r) {
                total = total + image.pixel(x, y).mean();
                n += 1.0;
            }
        }
    }
    total * (1.0 / n)
}

fn white_fur(density: f32) -> Material {
    Material {
        base_color: Spectrum::ONE,
        specular_weight: 0.0,
        fuzz_color: Spectrum::ONE,
        fuzz_roughness: 0.4,
        fuzz_density: density,
        fuzz_length: 0.12,
        fuzz_thickness: 0.004,
        fuzz_lean: 0.4,
        fuzz_curl: 0.4,
        fuzz_color_variation: 0.0,
        ..Material::default()
    }
}

/// A white ball under white fur, as thick as a felt's - several fibres
/// deep wherever the camera looks - is as white as the sky round it, over
/// its face and at its outline, given the bounces to get out.
#[test]
fn white_fur_on_a_white_ball_loses_no_light() {
    let deep = IntegratorConfig { max_bounces: 128, max_transmission_bounces: 128, ..IntegratorConfig::product() };
    let bare = furred_ball(white_fur(0.0), deep.clone());
    let furred = furred_ball(white_fur(0.4), deep);
    let (face, outline) = (centre(&furred).luminance(), rim(&furred).luminance());
    println!("bare {:.4} {:.4}, furred face {face:.4}, outline {outline:.4}", centre(&bare).luminance(), rim(&bare).luminance());
    assert!((centre(&bare).luminance() - 1.0).abs() < 0.02, "the bare control is off");
    assert!((face - 1.0).abs() < 0.03, "a white furred ball reads {face:.4} over its face: light is lost in the fur");
    assert!((outline - 1.0).abs() < 0.03, "a white furred ball reads {outline:.4} at its outline");
}

/// What the process holds privately, in bytes - its commit, which is what a
/// part's fur costs the machine - now, and at the most it has held.
#[cfg(windows)]
fn private_bytes() -> (usize, usize) {
    // PROCESS_MEMORY_COUNTERS_EX, field for field; most are not read.
    #[repr(C)]
    #[derive(Default)]
    #[allow(dead_code)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged_pool: usize,
        quota_paged_pool: usize,
        quota_peak_non_paged_pool: usize,
        quota_non_paged_pool: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(process: *mut std::ffi::c_void, counters: *mut Counters, size: u32) -> i32;
    }
    let mut counters = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Counters::default() };
    // SAFETY: the counters are sized as the call is told, and the pseudo
    // handle needs no closing.
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    (counters.private, counters.peak_pagefile)
}

#[cfg(not(windows))]
fn private_bytes() -> (usize, usize) {
    (0, 0)
}

/// What a sofa's seat cushion of velvet costs Tequila's renderer: 1.1 m²
/// at 400 fibres a square centimetre, 4.4 million as asked against the old
/// cap's 250,000 - the time to grow them and to build Embree's BVH over
/// them, what the fibres hold, and what the process holds more for them.
/// Run alone, `--ignored --test-threads=1`, so nothing else moves the
/// process's memory.
#[test]
#[ignore = "a measurement, not an assertion"]
fn measure_a_sofa_cushions_fibres() {
    use tequila_scene::fuzz::{grow_with, Growth};
    // A ball of 1.1 m² - a seat cushion's area - modelled at a metre and
    // scaled to its size.
    let ball = tequila_scene::demo::uv_sphere(1.0, 128, 64);
    let scale = (1.1 / (4.0 * std::f32::consts::PI)).sqrt();
    let velvet = FuzzRequest {
        length_bits: 0.003f32.to_bits(),
        density_bits: 400f32.to_bits(),
        thickness_bits: 0.00002f32.to_bits(),
        lean_bits: 0.35f32.to_bits(),
        curl_bits: 0.3f32.to_bits(),
        clump_bits: 0f32.to_bits(),
        comb_bits: 0f32.to_bits(),
        seed: 1,
    };
    let device = Device::shared().expect("an Embree device");
    for count in [250_000usize, 4_400_000] {
        let (before, _) = private_bytes();
        let start = std::time::Instant::now();
        let fur = grow_with(&ball, scale, &velvet, Growth { count: Some(count), paint: None }).expect("fibres");
        let grown = start.elapsed();
        let held = fur.fibres.as_ref().map_or(0, |f| f.bytes());
        let (after_growing, _) = private_bytes();
        let scene = SceneSnapshot::new(
            vec![Arc::new(fur)],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
            vec![Material::default()],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        );
        let start = std::time::Instant::now();
        let bvh = EmbreeIntersector::build(&device, scene).expect("builds");
        let built = start.elapsed();
        let (after_building, peak) = private_bytes();
        println!(
            "{count} fibres: grown in {:.0} ms, {:.0} bytes a fibre held ({:.0} MB, process +{:.0} MB); BVH built in {:.0} ms, process +{:.0} MB more ({:.0} bytes a fibre), at most +{:.0} MB while building",
            grown.as_secs_f64() * 1000.0,
            held as f64 / count as f64,
            held as f64 / 1e6,
            after_growing.saturating_sub(before) as f64 / 1e6,
            built.as_secs_f64() * 1000.0,
            after_building.saturating_sub(after_growing) as f64 / 1e6,
            after_building.saturating_sub(after_growing) as f64 / count as f64,
            peak.saturating_sub(after_growing) as f64 / 1e6,
        );
        drop(bvh);
    }
}
