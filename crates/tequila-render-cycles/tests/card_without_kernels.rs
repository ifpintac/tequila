//! A graphics card Cycles can open but not render on - one older than the
//! kernels shipped, or a driver too old to compile them - says so through
//! the session's status, rather than leaving its frames to never come.
//!
//! Made here by giving Cycles an empty kernel folder: the device opens, the
//! scene translates, and the render then has no GPU code to run, which is
//! what a work PC's card older than the shipped kernels' (or a driver older
//! than their PTX) meets. A process of its own, because Cycles reads its
//! kernel folder once per process.
//!
//! Skipped where Cycles is not installed, and unless
//! `TEQUILA_TEST_CYCLES_GPU` says there is a card to test with.

use tequila_core::math::Vec3;
use tequila_render_core::camera::Camera;
use tequila_render_cycles::{translate, FrameBuffers, Session};
use std::time::{Duration, Instant};

#[test]
fn a_card_that_cannot_render_says_so() {
    if let Err(why) = tequila_render_cycles::available() {
        eprintln!("skipped: {why}");
        return;
    }
    if std::env::var_os("TEQUILA_TEST_CYCLES_GPU").is_none() {
        eprintln!("skipped: TEQUILA_TEST_CYCLES_GPU is not set");
        return;
    }
    // Kernels by the shipped names, whose contents no card can load: Cycles
    // finds them, so the device opens, and fails when it comes to run them.
    let broken = std::env::temp_dir().join(format!("tequila-broken-kernels-{}", std::process::id()));
    let lib = broken.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    for name in [
        "kernel_sm_75.cubin.zst",
        "kernel_compute_75.ptx.zst",
        "kernel_optix.ptx.zst",
        "kernel_optix_shader_raytrace.ptx.zst",
        "kernel_optix_osl.ptx.zst",
        "kernel_optix_osl_services.ptx.zst",
        "kernel_optix_osl_camera.ptx.zst",
    ] {
        std::fs::write(lib.join(name), b"not a kernel").unwrap();
    }

    let mut session = match Session::live("OPTIX", 0, Some(&broken)) {
        Ok(session) => session,
        // Refusing at once is the case the viewport already handles.
        Err(why) => {
            eprintln!("refused at start, as handled: {why}");
            return;
        }
    };
    let scene = tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
    let view = translate::View {
        camera: Camera::look_at(Vec3::splat(4.0), Vec3::ZERO, Vec3::Y),
        width: 64,
        height: 48,
        transparent: false,
        ground_illumination: false,
        light_materials: true,
    };
    session.begin_edit();
    let built = translate::build_with(&mut session, &scene, &view, &mut translate::Images::default()).and_then(|_| {
        translate::configure(
            &mut session,
            &translate::Quality::with_lighting(true, false, None, &tequila_render_core::settings::IntegratorConfig::product()),
        )
    });
    session.end_edit(64, 48, 64);
    if let Err(why) = built {
        eprintln!("refused while translating, as handled: {why}");
        return;
    }

    let (mut frame, mut seen) = (FrameBuffers::default(), 0u64);
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut frames = 0;
    loop {
        if session.latest_frame(&mut frame, (64, 48), &mut seen).is_some() {
            frames += 1;
        }
        if let Err(why) = session.status() {
            println!("status: error: {why} (after {frames} frames)");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "a minute without a frame ({frames}) or an error: the viewport stays black and says nothing (status {:?})",
            session.status()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
