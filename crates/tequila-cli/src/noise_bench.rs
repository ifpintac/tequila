//! `--bench` with an engine: how long each renderer takes to make a picture
//! of a given cleanliness (`docs/07` R0.1).
//!
//! Equal samples are not equal noise. One renderer may spend twice as long
//! on a sample and make half the noise with it, so samples per second alone
//! says little about which picture is ready first. What the gates of
//! `docs/07` §1 compare is the time to *equal noise*.
//!
//! # How noise is measured
//!
//! The scene is rendered twice at the same sample count with two seeds, the
//! denoiser and adaptive sampling off. Both pictures are developed as the
//! application develops them (the project's exposure, tone map and display
//! encoding), and the noise is the root-mean-square difference between the
//! two, divided by √2: two independent renders of the same expectation
//! differ by √2 times the noise of either. No reference render is needed,
//! so the noise is each renderer's own, measured against itself, and a
//! renderer that gets the picture wrong is not rewarded or punished for it
//! here; `scripts/look-refs` is for that.
//!
//! In display values, not scene-linear ones, because that is what a person
//! judges as clean: a firefly ten times brighter than white counts once, as
//! the white it shows as, rather than a hundred times.
//!
//! Without adaptive sampling the noise of a path tracer falls about as one
//! over the square root of the samples, so the time to reach `TARGET` is the
//! render's time scaled by `(noise / TARGET)²`. About: measured on the
//! Headphones, both renderers' noise fell 5-12% faster than that for each
//! fourfold of samples (well-spread samples and firefly rejection both
//! help), so a time scaled from a few samples overstates by as much again.
//! So unless `-s` says how many, a short trial render at `TRIAL` samples
//! finds how many samples reach `TARGET`, and the measurement is made there,
//! where the scaling has almost nothing left to cover. The time used is the
//! second render's, made with everything already loaded; the first's, which
//! includes loading kernels and textures, is reported beside it.
//!
//! # What is left out of Cycles' picture
//!
//! The floor reflection the application lays over a Cycles render is
//! Tequila's renderer's work and takes no seed, so it would make the two
//! renders' noise look lower than it is. It is left out of both the picture
//! and the time, and the report says when a scene would have had one.

use super::*;
use tequila_render_core::scene::SceneSnapshot;
use tequila_render_core::settings::IntegratorConfig;

/// The noise counted as clean: 1% of full scale in display values, about
/// two and a half steps of an 8-bit picture.
pub const TARGET: f64 = 0.01;

/// The two seeds. Any two different numbers do; fixed so a run repeats.
const SEEDS: [u32; 2] = [1, 2];

/// The trial render's samples, and the most a measurement takes. A
/// measurement that lands more than a quarter off the target is made once
/// more where it says the target is.
const TRIAL: u32 = 16;
const MOST: u32 = 4096;

/// The samples that bring a trial's noise down to `TARGET`, by the square
/// root rule, in whole batches of eight, within [`TRIAL`, `MOST`].
pub fn samples_for_target(trial_samples: u32, trial_noise: f64) -> u32 {
    let wanted = trial_samples as f64 * (trial_noise / TARGET).powi(2);
    let batches = (wanted / 8.0).ceil().max(1.0) as u32;
    (batches * 8).clamp(TRIAL, MOST)
}

/// One measurement, as `--json` writes it.
pub struct Measurement {
    pub scene: String,
    pub engine: &'static str,
    pub device: String,
    pub width: usize,
    pub height: usize,
    pub samples: u32,
    pub triangles: usize,
    /// Making the scene ready to render: the BVH for Tequila's renderer; the
    /// session, the translation and its settings for Cycles.
    pub setup_seconds: f64,
    /// Each render's time, first then second.
    pub render_seconds: [f64; 2],
    pub noise: f64,
    /// The mean of the picture in display values, to tell a dark scene from
    /// a light one when reading the noise.
    pub mean: f64,
    pub peak_memory: Option<u64>,
    /// Whether the application would have laid its floor reflection over a
    /// Cycles render of this scene (left out here).
    pub floor_overlay: bool,
}

impl Measurement {
    pub fn samples_per_second(&self) -> f64 {
        (self.width * self.height) as f64 * self.samples as f64 / self.render_seconds[1]
    }

    pub fn seconds_to_target(&self) -> f64 {
        self.render_seconds[1] * (self.noise / TARGET).powi(2)
    }

    pub fn to_json(&self) -> String {
        let peak = self.peak_memory.map_or("null".to_string(), |b| b.to_string());
        format!(
            concat!(
                "{{\"scene\": {:?}, \"engine\": {:?}, \"device\": {:?}, \"width\": {}, \"height\": {}, ",
                "\"samples\": {}, \"triangles\": {}, \"setup_seconds\": {:.3}, \"render_seconds\": [{:.3}, {:.3}], ",
                "\"samples_per_second\": {:.0}, \"noise\": {:.6}, \"target\": {}, \"seconds_to_target\": {:.3}, ",
                "\"mean\": {:.5}, \"peak_memory\": {}, \"floor_overlay\": {}}}"
            ),
            self.scene,
            self.engine,
            self.device,
            self.width,
            self.height,
            self.samples,
            self.triangles,
            self.setup_seconds,
            self.render_seconds[0],
            self.render_seconds[1],
            self.samples_per_second(),
            self.noise,
            TARGET,
            self.seconds_to_target(),
            self.mean,
            peak,
            self.floor_overlay,
        )
    }
}

/// The root-mean-square difference of two developed pictures over √2, and
/// the mean of the first. Values are 16-bit display codes, read as [0, 1].
pub fn noise_of(a: &[u16], b: &[u16]) -> (f64, f64) {
    assert_eq!(a.len(), b.len(), "two pictures of different sizes");
    if a.is_empty() {
        return (0.0, 0.0);
    }
    let (mut squares, mut sum) = (0.0f64, 0.0f64);
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = (x as f64 / 65535.0, y as f64 / 65535.0);
        squares += (x - y) * (x - y);
        sum += x;
    }
    let n = a.len() as f64;
    ((squares / n).sqrt() / std::f64::consts::SQRT_2, sum / n)
}

/// `--bench NAME --engine E`: measure, print, and append to `--json`.
pub fn run(options: &Options, name: &str) -> Result<(), String> {
    let generated = BenchScene::from_name(name);
    let mut options = options.clone();
    if generated.is_none() {
        if !std::path::Path::new(name).is_file() {
            return Err(format!("{name:?} is neither a benchmark scene (simple_product, cad_assembly) nor a file"));
        }
        options.input = Some(name.to_string());
    }
    let document = match generated {
        Some(scene) => scene.build(),
        None => prepare(&options)?,
    };
    let project = generated.is_none() && is_project(&options);
    let settings = document.render.clone();
    let width = options.width.unwrap_or(960);
    let height = options.height.unwrap_or(540);

    tequila_materials::energy::prewarm();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (mut scene, _) = snapshot(&document, environment, &mut MeshCache::new());
    scene.background = environments.background(&document.environment);
    let triangles = scene.triangle_count();

    let aspect = width as f32 / height as f32;
    let camera = if project {
        document.cameras.active().camera
    } else if generated.is_some() {
        tequila_scene::bench::product_camera(&document, aspect)
    } else {
        framing(&options, &document, aspect).0
    };
    let mut style = document.image_style.clone();
    style = tequila_scene::lighting::developed_style(style, &camera);

    let mut config = document.lighting.config();
    config.spectral = settings.spectral;
    // Adaptive sampling off in both: it changes how noise falls with samples.
    config.adaptive_sampling_threshold = 0.0;

    let label = generated.map_or_else(|| file_label(name), |s| s.name().to_string());
    let cycles = options.cycles.unwrap_or(false);
    let device = if cycles {
        cycles_device(options.device.as_deref())?
    } else {
        if let Some(device) = options.device.as_deref().filter(|d| !d.eq_ignore_ascii_case("cpu")) {
            return Err(format!("Tequila's renderer runs on the processor only for now, not {device:?}"));
        }
        "CPU".to_string()
    };
    let reject_fireflies = if project { settings.reject_fireflies } else { true };
    let clamp = if project { settings.clamp_indirect } else { None };
    let picture = options.output_set.then_some(options.output.as_str());
    let measure = |samples: u32, picture: Option<&str>| {
        if cycles {
            with_cycles(&device, &scene, camera, width, height, samples, &config, project.then_some(&settings), &style, picture)
        } else {
            with_tequila(scene.clone(), camera, width, height, samples, config.clone(), reject_fireflies, clamp, &style, picture)
        }
    };
    let mut measurement = match options.samples {
        Some(samples) => measure(samples.max(1), picture)?,
        None => {
            let trial = measure(TRIAL, None)?;
            let samples = samples_for_target(TRIAL, trial.noise);
            println!("trial            {TRIAL} samples, noise {:.4}: measuring at {samples}", trial.noise);
            let first = measure(samples, picture)?;
            // Once more where the first landed far from the target, as it
            // does where fireflies make the noise fall slower than the rule.
            let off = first.noise / TARGET;
            let again = samples_for_target(samples, first.noise);
            if (off > 1.25 || off < 0.8) && again != samples {
                println!("                 noise {:.4} at {samples}: measuring again at {again}", first.noise);
                measure(again, picture)?
            } else {
                first
            }
        }
    };
    measurement.scene = label;
    measurement.triangles = triangles;
    measurement.peak_memory = peak_memory();

    report(&measurement);
    if let Some(path) = &options.json {
        use std::io::Write;
        make_parent(path)?;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("{path}: {e}"))?;
        writeln!(file, "{}", measurement.to_json()).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(())
}

fn report(m: &Measurement) {
    println!("scene            {}", m.scene);
    println!("engine           {} on {}", m.engine, m.device);
    println!("resolution       {}x{} at {} samples, {} triangles", m.width, m.height, m.samples, m.triangles);
    println!("setup            {:>8.2} s", m.setup_seconds);
    println!("render           {:>8.2} s, then {:.2} s", m.render_seconds[0], m.render_seconds[1]);
    println!("samples/s        {:>8.2} M", m.samples_per_second() / 1e6);
    println!("noise            {:>8.4}   (display values; clean is {TARGET})", m.noise);
    println!("time to clean    {:>8.1} s", m.seconds_to_target());
    if let Some(bytes) = m.peak_memory {
        println!("peak memory      {:>8.0} MB", bytes as f64 / 1e6);
    }
    if m.floor_overlay {
        println!("                 (the floor reflection laid over a Cycles render is left out)");
    }
}

/// A file's name without its folder or extension, as the report names it.
fn file_label(path: &str) -> String {
    std::path::Path::new(path).file_stem().map_or_else(|| path.to_string(), |s| s.to_string_lossy().into_owned())
}

/// "CPU", or "GPU" for the best card Cycles finds, or a kind by name.
fn cycles_device(asked: Option<&str>) -> Result<String, String> {
    let asked = asked.unwrap_or("CPU").to_ascii_uppercase();
    if asked != "GPU" {
        return Ok(asked);
    }
    let found = tequila_render_cycles::devices()?;
    tequila_render_cycles::best_of(found.iter().map(|(k, _, _)| k.as_str()))
        .map(str::to_string)
        .ok_or_else(|| "Cycles found no graphics card it can use".to_string())
}

#[allow(clippy::too_many_arguments)]
fn with_tequila(
    scene: SceneSnapshot,
    camera: Camera,
    width: usize,
    height: usize,
    samples: u32,
    config: IntegratorConfig,
    reject_fireflies: bool,
    clamp_sample: Option<f32>,
    style: &tequila_color::ImageStyle,
    picture: Option<&str>,
) -> Result<Measurement, String> {
    let device = Device::shared()?;
    let start = Instant::now();
    let intersector = EmbreeIntersector::build(&device, scene)?;
    let setup_seconds = start.elapsed().as_secs_f64();
    let display = tequila_color::DisplayPipeline::Builtin(style.display);

    let mut schedule = ProgressiveSchedule::default();
    schedule.update(60_000);
    let mut developed = Vec::new();
    let mut render_seconds = [0.0; 2];
    for (i, seed) in SEEDS.into_iter().enumerate() {
        let context = RenderContext { camera, config: config.clone(), scene_seed: seed, reject_fireflies, clamp_sample, region: None, ..Default::default() };
        let mut framebuffer = Framebuffer::new(width, height);
        let start = Instant::now();
        let mut done = 0;
        while done < samples {
            let batch = 8.min(samples - done);
            render_pass(&intersector, &mut framebuffer, &context, &schedule, batch);
            done += batch;
        }
        render_seconds[i] = start.elapsed().as_secs_f64();
        let linear: Vec<_> = (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).map(|(x, y)| framebuffer.pixel(x, y).mean().sanitized()).collect();
        if i == 0 {
            if let Some(path) = picture {
                finish_image(path, width, height, &linear, None, style, false)?;
            }
        }
        developed.push(style.develop_through_wide(width, height, &linear, &display));
    }
    let (noise, mean) = noise_of(&developed[0], &developed[1]);
    Ok(Measurement {
        scene: String::new(),
        engine: "tequila",
        device: "CPU".into(),
        width,
        height,
        samples,
        triangles: 0,
        setup_seconds,
        render_seconds,
        noise,
        mean,
        peak_memory: None,
        floor_overlay: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn with_cycles(
    device: &str,
    scene: &SceneSnapshot,
    camera: Camera,
    width: usize,
    height: usize,
    samples: u32,
    lighting: &IntegratorConfig,
    project: Option<&tequila_scene::render::RenderSettings>,
    style: &tequila_color::ImageStyle,
    picture: Option<&str>,
) -> Result<Measurement, String> {
    use tequila_render_cycles::{translate, Session};

    let start = Instant::now();
    let mut session = Session::new(device, 0, std::env::var_os("TEQUILA_CYCLES_KERNELS").map(std::path::PathBuf::from).as_deref())?;
    let view = translate::View {
        camera,
        width,
        height,
        transparent: false,
        ground_illumination: lighting.ground_illumination,
        light_materials: lighting.light_source_materials,
        light_groups: false,
        shadow_glass: lighting.shadow_glass(),
        manifold_caustics: lighting.manifold_caustics(),
    };
    let translation = translate::build(&mut session, scene, &view)?;
    let under = translation.background_under.then(|| translate::background(scene, &camera, width, height));
    let quality = match project {
        Some(settings) => translate::Quality::with_lighting(false, false, None, lighting)
            .with_clamping(settings.reject_fireflies, settings.clamp_indirect, false)
            .with_render_controls(settings.clamp_direct, settings.pixel_filter, settings.adaptive_threshold),
        None => translate::Quality::with_lighting(false, false, None, lighting),
    };
    translate::configure(&mut session, &quality)?;
    let setup_seconds = start.elapsed().as_secs_f64();
    let display = tequila_color::DisplayPipeline::Builtin(style.display);

    let mut developed = Vec::new();
    let mut render_seconds = [0.0; 2];
    for (i, seed) in SEEDS.into_iter().enumerate() {
        let integrator = session.scene_node("integrator")?;
        session.set_int(integrator, "seed", seed as i32)?;
        let start = Instant::now();
        let rgba = session.render(width, height, samples, |_, _| {})?;
        render_seconds[i] = start.elapsed().as_secs_f64();
        let linear: Vec<_> = rgba
            .chunks_exact(4)
            .enumerate()
            .map(|(i, px)| match &under {
                Some(background) => translate::lay_under(px, background[i]),
                None => translate::to_rendering(px[0], px[1], px[2]).sanitized(),
            })
            .collect();
        if i == 0 {
            if let Some(path) = picture {
                finish_image(path, width, height, &linear, None, style, false)?;
            }
        }
        developed.push(style.develop_through_wide(width, height, &linear, &display));
    }
    if developed[0] == developed[1] {
        return Err("Cycles made the same picture with both seeds, so its noise cannot be measured".into());
    }
    let (noise, mean) = noise_of(&developed[0], &developed[1]);
    Ok(Measurement {
        scene: String::new(),
        engine: "cycles",
        device: device.to_string(),
        width,
        height,
        samples,
        triangles: 0,
        setup_seconds,
        render_seconds,
        noise,
        mean,
        peak_memory: None,
        floor_overlay: translation.floor_reflection,
    })
}

/// The most memory this process has held: the peak working set on Windows,
/// the largest resident size elsewhere.
#[cfg(windows)]
pub fn peak_memory() -> Option<u64> {
    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut ProcessMemoryCounters, size: u32) -> i32;
    }
    let mut counters = ProcessMemoryCounters { cb: std::mem::size_of::<ProcessMemoryCounters>() as u32, ..Default::default() };
    let size = counters.cb;
    // SAFETY: a PROCESS_MEMORY_COUNTERS with its size set, for this process's
    // pseudo-handle, which needs no closing.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    (ok != 0).then_some(counters.peak_working_set_size as u64)
}

#[cfg(unix)]
pub fn peak_memory() -> Option<u64> {
    #[repr(C)]
    struct Rusage {
        utime: [i64; 2],
        stime: [i64; 2],
        maxrss: std::ffi::c_long,
        rest: [std::ffi::c_long; 13],
    }
    extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    let mut usage = Rusage { utime: [0; 2], stime: [0; 2], maxrss: 0, rest: [0; 13] };
    // SAFETY: RUSAGE_SELF (0) fills a `struct rusage`, laid out as this is on
    // the 64-bit systems Tequila builds for.
    let failed = unsafe { getrusage(0, &mut usage) };
    // Bytes on a Mac, kilobytes on Linux.
    let scale = if cfg!(target_os = "macos") { 1 } else { 1024 };
    (failed == 0).then(|| usage.maxrss as u64 * scale)
}

#[cfg(not(any(windows, unix)))]
pub fn peak_memory() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_equal_pictures_have_no_noise() {
        let a = vec![1000u16, 20000, 65535, 0];
        assert_eq!(noise_of(&a, &a).0, 0.0);
    }

    #[test]
    fn the_noise_is_the_difference_over_root_two() {
        // Every value one hundredth of full scale apart: the difference's
        // RMS is 0.01, so each picture's own noise is 0.01 / √2.
        let step = (0.01 * 65535.0f64).round() as u16;
        let a = vec![30000u16; 64];
        let b: Vec<u16> = (0..64).map(|i| if i % 2 == 0 { 30000 + step } else { 30000 - step }).collect();
        let (noise, mean) = noise_of(&a, &b);
        let expected = step as f64 / 65535.0 / std::f64::consts::SQRT_2;
        assert!((noise - expected).abs() < 1e-12, "{noise} against {expected}");
        assert!((mean - 30000.0 / 65535.0).abs() < 1e-12);
    }

    #[test]
    fn time_to_clean_scales_with_the_square_of_the_noise() {
        let m = Measurement {
            scene: "s".into(),
            engine: "tequila",
            device: "CPU".into(),
            width: 10,
            height: 10,
            samples: 64,
            triangles: 0,
            setup_seconds: 0.0,
            render_seconds: [9.0, 4.0],
            noise: TARGET * 3.0,
            mean: 0.5,
            peak_memory: None,
            floor_overlay: false,
        };
        // Three times too noisy takes nine times the second render's time.
        assert!((m.seconds_to_target() - 36.0).abs() < 1e-9);
        assert!((m.samples_per_second() - 1600.0).abs() < 1e-9);
        assert!(m.to_json().contains("\"seconds_to_target\": 36.000"));
    }

    #[test]
    fn the_trial_says_how_many_samples_reach_the_target() {
        // Twice the target's noise at 16 samples wants four times as many.
        assert_eq!(samples_for_target(16, TARGET * 2.0), 64);
        // Rounded up to a whole batch of eight.
        assert_eq!(samples_for_target(16, TARGET * 1.1), 24);
        // Never fewer than the trial, nor more than the most.
        assert_eq!(samples_for_target(16, TARGET * 0.5), TRIAL);
        assert_eq!(samples_for_target(16, TARGET * 100.0), MOST);
    }

    #[test]
    fn peak_memory_is_read() {
        assert!(peak_memory().is_some_and(|b| b > 1_000_000));
    }
}
