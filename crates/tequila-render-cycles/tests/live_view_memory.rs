//! A live view on a Mac's graphics card keeps fewer path states than Cycles
//! 5.0 gives a session on Metal - four million, eight where memory allows,
//! a third to half a kilobyte each - and each session keeps its own number:
//! Cycles chose it once, for every session of the process, by whichever came
//! first. Measured as the process's footprint, which on a Mac counts what is
//! on the graphics card. A process of its own, for that reason.
//!
//! Skipped where Cycles is not installed, on anything but a Mac, and unless
//! `TEQUILA_TEST_CYCLES_GPU` says there is a card to test with.

#[cfg(target_os = "macos")]
#[test]
fn a_live_view_on_a_mac_keeps_fewer_path_states_and_each_its_own() {
    if let Err(why) = tequila_render_cycles::available() {
        eprintln!("skipped: {why}");
        return;
    }
    if std::env::var_os("TEQUILA_TEST_CYCLES_GPU").is_none() {
        eprintln!("skipped: TEQUILA_TEST_CYCLES_GPU is not set");
        return;
    }
    const GB: f64 = (1u64 << 30) as f64;
    // From the fewest path states to the most, each view closed before the
    // next: memory a view let go may not have come back yet when the next
    // one starts, so each is measured from where the process began.
    let start = footprint();
    let few = live_view(Some("65536")).saturating_sub(start) as f64 / GB;
    let tequilas = live_view(None).saturating_sub(start) as f64 / GB;
    let cycles_own = live_view(Some("0")).saturating_sub(start) as f64 / GB;
    eprintln!(
        "the footprint with a live view open: {few:.2} GB more with 65,536 path states, {tequilas:.2} GB with Tequila's number, {cycles_own:.2} GB with Cycles' own"
    );
    assert!(
        tequilas - few > 0.2,
        "a live view kept as few path states as the one before it asked for ({tequilas:.2} GB against {few:.2} GB): the first session's number was every session's"
    );
    // So many cores that Tequila's number is no fewer than Cycles' eight
    // million, as on an Ultra, keep Cycles' own.
    if gpu_cores() * 131_072 < 8_388_608 {
        assert!(
            cycles_own - tequilas > 0.5,
            "a live view kept nearly as many path states as Cycles gives a session ({tequilas:.2} GB against {cycles_own:.2} GB)"
        );
    }
}

/// The process's footprint while a live view of the demo scene is open and
/// has rendered a few samples, with `TEQUILA_CYCLES_LIVE_STATES` as given (or
/// unset). The view is closed before returning.
#[cfg(target_os = "macos")]
fn live_view(states: Option<&str>) -> u64 {
    use std::time::{Duration, Instant};
    use tequila_core::math::Vec3;
    use tequila_render_core::camera::Camera;
    use tequila_render_cycles::{translate, FrameBuffers, Session};

    match states {
        Some(states) => std::env::set_var("TEQUILA_CYCLES_LIVE_STATES", states),
        None => std::env::remove_var("TEQUILA_CYCLES_LIVE_STATES"),
    }
    let mut session = Session::live("METAL", 0, None).expect("a live session on the graphics card");
    let scene = tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
    let (width, height) = (640, 360);
    let view = translate::View {
        camera: Camera::look_at(Vec3::splat(4.0), Vec3::ZERO, Vec3::Y),
        width,
        height,
        transparent: false,
        ground_illumination: false,
        light_materials: true,
        light_groups: false,
        shadow_glass: tequila_render_core::settings::IntegratorConfig::product().shadow_glass(),
        manifold_caustics: false,
    };
    session.begin_edit();
    let built = translate::build_with(&mut session, &scene, &view, &mut translate::Images::default()).and_then(|_| {
        translate::configure(
            &mut session,
            &translate::Quality::with_lighting(true, false, None, &tequila_render_core::settings::IntegratorConfig::product()),
        )
    });
    session.end_edit(width, height, 64);
    built.expect("the demo scene translates");

    // The path states are made before the first sample; a frame with samples
    // comes after them.
    let (mut frame, mut seen) = (FrameBuffers::default(), 0u64);
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if session.latest_frame(&mut frame, (width, height), &mut seen).is_some_and(|frame| frame.samples >= 8) {
            break;
        }
        if let Err(why) = session.status() {
            panic!("Cycles stopped: {why}");
        }
        assert!(Instant::now() < deadline, "two minutes without a frame of 8 samples");
        std::thread::sleep(Duration::from_millis(20));
    }
    let open = footprint();
    drop(session);
    open
}

/// The process's footprint in bytes, as Activity Monitor's Memory column
/// and `footprint` count it: on a Mac, with what it has on the graphics card.
#[cfg(target_os = "macos")]
fn footprint() -> u64 {
    // `rusage_info_v0`: a 16-byte UUID, then ten 64-bit counts, the eighth
    // the footprint.
    #[repr(C)]
    struct RusageInfo {
        uuid: [u8; 16],
        counts: [u64; 10],
    }
    extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfo) -> i32;
    }
    let mut info = RusageInfo { uuid: [0; 16], counts: [0; 10] };
    // RUSAGE_INFO_V0 is flavour 0.
    let result = unsafe { proc_pid_rusage(std::process::id() as i32, 0, &mut info) };
    assert_eq!(result, 0, "proc_pid_rusage failed");
    info.counts[7]
}

/// The graphics card's cores, as the IO registry has them (0 if it does not
/// say).
#[cfg(target_os = "macos")]
fn gpu_cores() -> u64 {
    let listed = std::process::Command::new("ioreg").args(["-rd1", "-c", "AGXAccelerator"]).output();
    let text = listed.map(|out| String::from_utf8_lossy(&out.stdout).into_owned()).unwrap_or_default();
    text.split("\"gpu-core-count\" = ").nth(1).and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}
