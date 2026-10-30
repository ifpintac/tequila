//! Running the render queue (`docs/04-UX-SPEC.md` §11).
//!
//! A thread of its own, and not the viewport's. The two have opposite jobs: the
//! viewport wants a picture *now* and will happily throw away everything it has
//! to show a camera move, while a queued render wants to finish and must not be
//! disturbed by anyone orbiting. Sharing one thread would mean one of those
//! behaviours losing, and the loser is always the overnight render.
//!
//! Everything crosses the boundary as a message. The job carries its own
//! snapshot of the scene, so a user editing the document while a queue runs
//! changes what the *next* job renders and not what the current one is halfway
//! through — which is the behaviour anyone who has queued a render expects and
//! the opposite of what a shared document would give.

use tequila_color::{DisplayPipeline, ImageStyle};
use tequila_format::output::{
    AlphaMode as OutputAlphaMode, Metadata, NameFields, RenderOutput,
};
use tequila_render_core::aov::AovSet;
use tequila_render_core::camera::Camera;
use tequila_render_core::framebuffer::{AovBuffers, Framebuffer};
use tequila_render_core::scene::SceneSnapshot;
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass_with_aovs, Device, EmbreeIntersector, RenderContext};
use tequila_scene::render::{AlphaMode, Halt, HiddenLines, RenderEngine, RenderKind, RenderSettings};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Everything one queued render needs.
///
/// Carries its own scene rather than a reference to the project's: a job that
/// read the live document would render whatever the user happened to be doing
/// while it ran.
pub struct BatchJob {
    pub id: u64,
    pub name: String,
    pub scene: Arc<SceneSnapshot>,
    /// Finish a pending environment build on the render worker, never the UI.
    pub pending_environment: Option<tequila_scene::document::EnvironmentSettings>,
    pub camera: Camera,
    pub settings: RenderSettings,
    pub config: IntegratorConfig,
    pub style: ImageStyle,
    pub display: DisplayPipeline,
    /// Names for the Cryptomatte manifest, by instance and material index.
    pub object_names: Vec<String>,
    pub material_names: Vec<String>,
    pub fields: NameFields,
    /// Render layers: a name and the snapshot that layer is rendered from,
    /// each written beside the main image once it is done.
    pub layers: Vec<(String, Arc<SceneSnapshot>)>,
}

/// What the renderer says about a job.
pub enum BatchEvent {
    Stage { id: u64, label: &'static str },
    Preview { id: u64, width: usize, height: usize, rgba: Vec<u8> },
    Started {
        id: u64,
    },
    Progress {
        id: u64,
        samples: u32,
        seconds: f64,
    },
    Finished {
        id: u64,
        path: String,
        width: usize,
        height: usize,
        samples: u32,
        seconds: f64,
        /// The developed image, for the gallery.
        preview: Vec<u8>,
        halt: Halt,
        stopped: bool,
        /// An image per light group, when the render was asked to keep them.
        relight: Option<Box<Relight>>,
    },
    Failed {
        id: u64,
        message: String,
    },
}

/// Owns the batch thread.
pub struct BatchRenderer {
    jobs: Sender<BatchJob>,
    events: Receiver<BatchEvent>,
    cancel: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    preview: Arc<Mutex<Option<BatchEvent>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for BatchRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchRenderer").finish_non_exhaustive()
    }
}

impl Default for BatchRenderer {
    fn default() -> Self {
        BatchRenderer::new()
    }
}

impl BatchRenderer {
    pub fn new() -> BatchRenderer {
        let (job_sender, job_receiver) = channel::<BatchJob>();
        let (event_sender, event_receiver) = channel::<BatchEvent>();
        let cancel = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(false));
        let preview = Arc::new(Mutex::new(None));
        let thread_preview = preview.clone();

        let thread_cancel = cancel.clone();
        let thread_running = running.clone();
        let thread = std::thread::Builder::new()
            .name("tequila-batch".into())
            .spawn(move || {
                let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).saturating_sub(2).max(1);
                let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().ok();
                // The channel closing is how the thread is told to stop, which
                // means dropping the renderer is all the shutdown there is.
                while let Ok(job) = job_receiver.recv() {
                    thread_running.store(true, Ordering::Release);
                    let id = job.id;
                    let _ = event_sender.send(BatchEvent::Started { id });
                    let work = || run(job, &event_sender, &thread_cancel, &thread_preview);
                    let result = match &pool { Some(pool) => pool.install(work), None => work() };
                    match result {
                        Ok(finished) => {
                            let _ = event_sender.send(finished);
                        }
                        Err(message) => {
                            let _ = event_sender.send(BatchEvent::Failed { id, message });
                        }
                    }
                    thread_running.store(false, Ordering::Release);
                }
            })
            .ok();

        BatchRenderer {
            jobs: job_sender,
            events: event_receiver,
            cancel,
            running,
            preview,
            thread,
        }
    }

    pub fn submit(&self, job: BatchJob) {
        self.cancel.store(false, Ordering::Release);
        self.running.store(true, Ordering::Release);
        if self.jobs.send(job).is_err() { self.running.store(false, Ordering::Release); }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// Stop the job in progress. It still reports, with whatever samples it
    /// reached — a cancelled render is not a failed one, and throwing away
    /// forty minutes of work because someone changed their mind about the last
    /// ten would be its own kind of failure.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    /// Everything the renderer has said since the last call. Never blocks.
    pub fn drain(&self) -> Vec<BatchEvent> {
        let mut events = Vec::new();
        if let Some(preview) = self.preview.lock().unwrap().take() { events.push(preview); }
        events.extend(self.events.try_iter());
        events
    }
}

impl Drop for BatchRenderer {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        // Dropping the sender closes the channel, which ends the loop.
        let (dead, _) = channel();
        let _ = std::mem::replace(&mut self.jobs, dead);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Unsampled pixels outside a crop must not hold its progress at zero.
fn region_progress(frame: &Framebuffer, region: Option<tequila_render_core::scheduler::Tile>) -> (u32, bool) {
    let Some(r) = region else { return (frame.min_samples(), frame.converged_fraction() >= 1.0); };
    let mut minimum = u32::MAX;
    let mut maximum = 0;
    let mut converged = true;
    for y in r.y0..r.y1.min(frame.height) { for x in r.x0..r.x1.min(frame.width) {
        let pixel = frame.pixel(x, y);
        maximum = maximum.max(pixel.samples);
        if !pixel.retired { minimum = minimum.min(pixel.samples); converged = false; }
    }}
    (if converged { maximum } else { minimum }, converged)
}

fn run(
    mut job: BatchJob,
    events: &Sender<BatchEvent>,
    cancel: &AtomicBool,
    preview_slot: &Mutex<Option<BatchEvent>>,
) -> Result<BatchEvent, String> {
    let _ = events.send(BatchEvent::Stage { id: job.id, label: "Preparing scene" });
    let settings = &job.settings;
    if settings.width == 0 || settings.height == 0 || settings.max_samples == 0 {
        return Err("Render dimensions and sample count must be greater than zero".into());
    }
    if let Some(r) = settings.region {
        if r.width == 0 || r.height == 0 || r.x >= settings.width || r.y >= settings.height {
            return Err("The render region does not contain any image pixels".into());
        }
    }
    if settings.kind == RenderKind::Wireframe {
        return render_wireframe(&job, events);
    }
    if let Some(environment) = job.pending_environment.take() {
        let mut cache = tequila_scene::environment::EnvironmentCache::new();
        let lighting = cache.environment(&environment);
        let background = cache.background(&environment);
        let scene = Arc::make_mut(&mut job.scene);
        scene.environment = lighting.clone();
        scene.background = background.clone();
        for (_, layer) in &mut job.layers {
            let layer = Arc::make_mut(layer);
            layer.environment = lighting.clone();
            layer.background = background.clone();
        }
    }
    let (engine, why) = engine_for(&settings, !job.layers.is_empty());
    if let Some(why) = why {
        eprintln!("Tequila: {why}");
    }
    if engine.is_cycles() {
        let frame = match render_cycles(&job, engine, events, cancel) {
            // A graphics card Cycles could open but not render on - older
            // than the kernels shipped, or with a driver too old for them -
            // failed the render; the processor can make it. Said in the
            // queue, and the render goes on there. Only the render: a folder
            // that cannot be made or a full disk, after it, is not the
            // card's doing, and rendering again on the processor for hours
            // only to fail the same way helped nobody.
            Err(why) if engine == RenderEngine::CyclesGpu && !cancel.load(Ordering::Acquire) => {
                eprintln!("Tequila: Cycles could not render on the graphics card, so it renders on the processor: {why}");
                let _ = events.send(BatchEvent::Stage { id: job.id, label: "The graphics card failed: rendering on the processor" });
                render_cycles(&job, RenderEngine::CyclesCpu, events, cancel)?
            }
            other => other?,
        };
        return finish_cycles(&job, frame, events, cancel);
    }
    let device = Device::shared().map_err(|e| e.to_string())?;
    let intersector =
        EmbreeIntersector::build(&device, (*job.scene).clone()).map_err(|e| e.to_string())?;

    let mut framebuffer = Framebuffer::new(settings.width, settings.height);
    let want_passes = settings.effective_write_passes();
    let set = if want_passes {
        AovSet::standard_for(&job.scene)
    } else if settings.relight {
        AovSet::light_groups_for(&job.scene)
    } else {
        AovSet::default()
    };
    let mut aovs = (want_passes || settings.relight).then(|| {
        let mut buffers = AovBuffers::new(settings.width, settings.height, &set);
        buffers.name_things(&job.object_names, &job.material_names);
        buffers
    });

    let mut config = job.config.clone();
    // The render's pixel filter, a box of one pixel unless it asks for another.
    config.pixel_filter = settings.pixel_filter.unwrap_or(tequila_render_core::settings::PixelFilter::BOX);
    config.adaptive_sampling_threshold = if settings.adaptive {
        settings.adaptive_threshold.unwrap_or(config.adaptive_sampling_threshold)
    } else {
        // Zero retires nothing, which is how "adaptive off" is expressed
        // without the renderer needing a second switch.
        0.0
    };
    let context = RenderContext {
        camera: job.camera,
        config,
        scene_seed: 0,
        reject_fireflies: settings.reject_fireflies,
        clamp_sample: settings.clamp_indirect,
        region: settings.region.map(|r| tequila_render_core::scheduler::Tile {
            x0: r.x,
            y0: r.y,
            x1: r.x.saturating_add(r.width).min(settings.width),
            y1: r.y.saturating_add(r.height).min(settings.height),
        }),
        ..RenderContext::default()
    };

    // A schedule that is never interactive: a queued render has no camera
    // moving, so every pass is a full-resolution one.
    let mut schedule = ProgressiveSchedule::default();
    schedule.update(60_000);

    let started = Instant::now();
    let halt;
    // In batches rather than one sample at a time, so the progress report and
    // the cancel check cost nothing measurable next to the rendering.
    const BATCH: u32 = 2;
    let mut last_preview = Instant::now();
    let _ = events.send(BatchEvent::Stage { id: job.id, label: "Rendering" });
    loop {
        let (samples, converged) = region_progress(&framebuffer, context.region);
        let elapsed = started.elapsed();
        if let Some(reason) = settings.stop_after(samples, elapsed, converged) {
            halt = reason;
            break;
        }
        if cancel.load(Ordering::Acquire) {
            // A cancelled render is not a failed one — it stops for the same
            // reason a time limit stops it, and it still writes what it has.
            halt = Halt::Time;
            break;
        }

        let remaining = settings.max_samples.saturating_sub(samples).min(BATCH).max(1);
        render_pass_with_aovs(
            &intersector,
            &mut framebuffer,
            aovs.as_mut().map(|buffers| (buffers, &set)),
            &context,
            &schedule,
            remaining,
        );
        let _ = events.send(BatchEvent::Progress {
            id: job.id,
            samples: region_progress(&framebuffer, context.region).0,
            seconds: started.elapsed().as_secs_f64(),
        });
        if samples == 0 || last_preview.elapsed().as_secs_f32() >= 1.0 {
            // Large enough to fill the Render window's stage without looking soft.
            let width = settings.width.min(1280);
            let height = (settings.height * width / settings.width).max(1);
            let mut color = Vec::with_capacity(width * height);
            let mut alpha = Vec::with_capacity(width * height);
            for y in 0..height { for x in 0..width {
                let p = framebuffer.pixel(x * settings.width / width, y * settings.height / height);
                color.push(p.mean_foreground()); alpha.push(p.alpha());
            }}
            let output = RenderOutput { width, height, color, alpha, layers: Vec::new(),
                style: job.style, display: job.display.clone(), alpha_mode: OutputAlphaMode::Straight };
            *preview_slot.lock().unwrap() = Some(BatchEvent::Preview { id: job.id, width, height, rgba: output.to_rgba8() });
            last_preview = Instant::now();
        }
    }

    let samples = region_progress(&framebuffer, context.region).0;
    let seconds = started.elapsed().as_secs_f64();
    let stopped = cancel.load(Ordering::Acquire);
    if stopped && samples == 0 { return Err("Cancelled before any samples were rendered".into()); }
    let _ = events.send(BatchEvent::Stage { id: job.id, label: if settings.denoise { "Denoising" } else { "Saving image" } });
    if let Some(buffers) = &mut aovs {
        buffers.finish(samples);
    }

    // ---- The file ------------------------------------------------------
    let (path, format) = output_file(&job, samples)?;

    let (color, alpha) = develop(&framebuffer, settings, settings.alpha_mode.transparent_background());
    let (color, alpha) = if settings.alpha_mode == AlphaMode::Silhouette {
        let _ = events.send(BatchEvent::Stage { id: job.id, label: "Cutting out" });
        tequila_render_cpu::Silhouette::trace(&intersector, &context.camera, settings.width, settings.height, SILHOUETTE_SAMPLES, context.scene_seed)
            .cut(&color)
    } else {
        (color, alpha)
    };

    let output = RenderOutput {
        width: settings.width,
        height: settings.height,
        color,
        alpha,
        layers: if want_passes { layers_of(aovs.as_ref(), &set, samples, settings) } else { Vec::new() },
        style: job.style,
        display: job.display.clone(),
        alpha_mode: output_alpha(settings),
    };
    let preview = output.to_rgba8();
    let relight = match (&aovs, settings.relight) {
        (Some(buffers), true) => {
            let _ = events.send(BatchEvent::Stage { id: job.id, label: "Keeping light groups" });
            Some(Box::new(Relight::gather(&framebuffer, buffers, &set, samples, settings, &output, path.clone(), format)))
        }
        _ => None,
    };
    let _ = events.send(BatchEvent::Stage { id: job.id, label: "Saving image" });

    // An EXR asked for passes gets the full multi-layer file, with the
    // Cryptomatte metadata the other formats have nowhere to put.
    match format {
        tequila_format::output::ImageFormat::Exr(precision) if want_passes => {
            // Its beauty and alpha as every other file gets them - denoised,
            // in the alpha mode chosen, cut out, at the precision asked for -
            // not the framebuffer's raw mean and coverage, which a file with
            // passes used to carry while the preview showed the finished one.
            let mut image = tequila_format::passes::exr_from_render(&framebuffer, aovs.as_ref(), &set);
            let (color, alpha) = output.matted();
            for layer in &mut image.layers {
                match layer.name.as_str() {
                    "" => *layer = tequila_format::exr::Layer { name: String::new(), channels: tequila_format::exr::Channels::Color(color.clone()), precision },
                    "A" => *layer = tequila_format::exr::Layer { name: "A".into(), channels: tequila_format::exr::Channels::Scalar(alpha.clone()), precision },
                    _ => {}
                }
            }
            image.write(&path).map_err(|e| e.to_string())?;
        }
        _ => output.write(&path, format).map_err(|e| e.to_string())?,
    }

    write_sidecars(&path, &job, settings, samples, seconds, halt)?;

    // ---- Render layers -------------------------------------------------
    //
    // Each a render of its own, to the samples the main image reached, so
    // the layers match it in noise and composite back without one standing
    // out. Written beside it as `<image>_<layer>`.
    for (name, scene) in &job.layers {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let _ = events.send(BatchEvent::Stage { id: job.id, label: "Rendering layers" });
        let intersector = EmbreeIntersector::build(&device, (**scene).clone()).map_err(|e| e.to_string())?;
        let mut layer = Framebuffer::new(settings.width, settings.height);
        let target = samples.max(1);
        while region_progress(&layer, context.region).0 < target && !region_progress(&layer, context.region).1 {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            let remaining = (target - region_progress(&layer, context.region).0).min(BATCH).max(1);
            render_pass_with_aovs(&intersector, &mut layer, None, &context, &schedule, remaining);
        }
        // A layer always carries alpha, so its colour is its foreground.
        let (color, alpha) = develop(&layer, settings, true);
        let output = RenderOutput {
            width: settings.width,
            height: settings.height,
            color,
            alpha,
            layers: Vec::new(),
            style: job.style,
            display: job.display.clone(),
            // A layer is for compositing, and is nothing without its
            // coverage: an opaque main image still gets layers with alpha.
            alpha_mode: match output_alpha(settings) {
                OutputAlphaMode::None => OutputAlphaMode::Premultiplied,
                other => other,
            },
        };
        output
            .write(&layer_path(&path, name), format)
            .map_err(|e| e.to_string())?;
    }

    Ok(BatchEvent::Finished {
        id: job.id,
        path: path.to_string_lossy().to_string(),
        width: settings.width,
        height: settings.height,
        samples,
        seconds: started.elapsed().as_secs_f64(),
        preview,
        halt,
        stopped: stopped || cancel.load(Ordering::Acquire),
        relight,
    })
}

/// Where a finished job's image goes, and in what format: the render's
/// folder, made if need be, and a name from its template that no file has.
fn output_file(job: &BatchJob, samples: u32) -> Result<(std::path::PathBuf, tequila_format::output::ImageFormat), String> {
    let settings = &job.settings;
    let directory = if settings.output_directory.trim().is_empty() {
        crate::render_window::fallback_directory()
    } else {
        std::path::PathBuf::from(settings.output_directory.trim())
    };
    std::fs::create_dir_all(&directory)
        .map_err(|e| format!("{} could not be made: {e}", directory.display()))?;

    let mut fields = job.fields.clone();
    fields.width = settings.width;
    fields.height = settings.height;
    fields.samples = samples;
    let format = crate::render_window::image_format(settings);
    let path = tequila_format::output::first_free(tequila_format::output::output_path(
        &directory,
        &settings.filename_template,
        &fields,
        format,
    ));
    Ok((path, format))
}

/// A wireframe of `scene` from `camera` as its file will hold it, drawn by
/// `tequila_render_cpu::wireframe` from the lines `tequila_scene::wireframe`
/// gathers. Developed through plain sRGB with no look: a drawing is not
/// lit, and its colours are the ones it was given.
pub(crate) fn wireframe_output(scene: &SceneSnapshot, camera: &Camera, settings: &RenderSettings) -> Result<RenderOutput, String> {
    let (width, height) = (settings.width, settings.height);
    let look = &settings.wireframe;
    let lines = tequila_scene::wireframe::lines(scene, camera, look.edges);
    let coverage = if lines.is_empty() {
        tequila_render_cpu::wireframe::Coverage { width, height, front: vec![0.0; width * height], behind: vec![0.0; width * height], strokes: 0 }
    } else {
        let device = Device::shared().map_err(|e| e.to_string())?;
        let intersector = EmbreeIntersector::build(&device, tequila_scene::wireframe::visible_scene(scene)).map_err(|e| e.to_string())?;
        tequila_render_cpu::wireframe::draw(&intersector, camera, width, height, &lines, look.line_width, look.hidden != HiddenLines::Hidden)?
    };
    let (color, alpha) = tequila_scene::wireframe::composite(
        width,
        height,
        &coverage.front,
        &coverage.behind,
        look,
        settings.alpha_mode.transparent_background(),
    );
    let srgb = tequila_color::DisplayTransform::Srgb;
    Ok(RenderOutput {
        width,
        height,
        color,
        alpha,
        layers: Vec::new(),
        style: ImageStyle { display: srgb, ..ImageStyle::default() },
        display: DisplayPipeline::Builtin(srgb),
        alpha_mode: output_alpha(settings),
    })
}

/// A wireframe job: drawn in a second or two rather than rendered, and
/// written through the same output, file name and sidecars as a render.
fn render_wireframe(job: &BatchJob, events: &Sender<BatchEvent>) -> Result<BatchEvent, String> {
    let started = Instant::now();
    let _ = events.send(BatchEvent::Stage { id: job.id, label: "Drawing the wireframe" });
    let output = wireframe_output(&job.scene, &job.camera, &job.settings)?;
    let _ = events.send(BatchEvent::Stage { id: job.id, label: "Saving image" });
    let (path, format) = output_file(job, 0)?;
    output.write(&path, format).map_err(|e| e.to_string())?;
    let seconds = started.elapsed().as_secs_f64();
    write_sidecars(&path, job, &job.settings, 0, seconds, Halt::Samples)?;
    Ok(BatchEvent::Finished {
        id: job.id,
        path: path.to_string_lossy().to_string(),
        width: job.settings.width,
        height: job.settings.height,
        samples: 0,
        seconds,
        preview: output.to_rgba8(),
        halt: Halt::Samples,
        stopped: false,
        relight: None,
    })
}

/// The same job rendered by Cycles: translated, rendered on the processor or
/// the graphics card, and written through the same output and sidecars.
///
/// What Cycles cannot do here yet is said rather than skipped silently:
/// passes, light groups and render layers come from Tequila's own renderer.
/// Whether Cycles is installed here, and if it is, whether it found a
/// graphics card it can render on. Asked once.
pub(crate) fn cycles_here() -> &'static Result<bool, String> {
    static CYCLES: std::sync::OnceLock<Result<bool, String>> = std::sync::OnceLock::new();
    CYCLES.get_or_init(|| {
        // The kinds the graphics-card engine renders on
        // (`RenderEngine::cycles_device`): Metal on a Mac, NVIDIA's or AMD's
        // elsewhere.
        let usable = |kind: &str| if cfg!(target_os = "macos") { kind == "METAL" } else { matches!(kind, "CUDA" | "OPTIX" | "HIP") };
        tequila_render_cycles::devices().map(|devices| devices.iter().any(|(kind, _, _)| usable(kind)))
    })
}

/// The engine that renders `settings`, and why, when it is not the one they
/// name. Cycles renders the image alone, so passes, light groups, layers
/// and regions are Tequila's renderer's to make; without Cycles installed,
/// so is everything; and without a graphics card Cycles can use, Cycles
/// renders on the processor.
pub(crate) fn engine_for(settings: &RenderSettings, layers: bool) -> (RenderEngine, Option<String>) {
    let engine = settings.engine;
    if !engine.is_cycles() {
        return (engine, None);
    }
    if layers || settings.region.is_some() {
        return (RenderEngine::Tequila, Some("layers and regions are rendered by Tequila's own renderer".into()));
    }
    if (settings.effective_write_passes() || settings.relight) && !tequila_render_cycles::has_passes() {
        return (RenderEngine::Tequila, Some("this Cycles library renders the image alone, so passes and light groups are Tequila's own renderer's".into()));
    }
    // Its passes are Cycles' own, Tequila's names on them, short of those
    // only Tequila's renderer makes.
    let without = settings.effective_write_passes().then(|| format!("from Cycles, a render's passes leave out {TEQUILA_ONLY_PASSES}, which Tequila's own renderer writes"));
    match cycles_here() {
        Err(why) => (RenderEngine::Tequila, Some(format!("Cycles is not available ({why}), so Tequila's own renderer renders"))),
        Ok(false) if engine == RenderEngine::CyclesGpu => {
            (RenderEngine::CyclesCpu, Some("Cycles found no graphics card it can use, so it renders on the processor".into()))
        }
        Ok(_) => (engine, without),
    }
}

/// What Cycles rendered, before Tequila lays its floor reflection over it
/// and saves it.
struct CyclesFrame {
    color: Vec<tequila_core::spectrum::Spectrum>,
    alpha: Vec<f32>,
    /// The passes it kept, for a render that asked for them.
    passes: Option<CyclesPasses>,
    transparent: bool,
    floor_reflection: bool,
    started: Instant,
    seconds: f64,
    rendered: u32,
    halt: Halt,
    stopped: bool,
}

/// The passes a Cycles render keeps, in Tequila's terms: its depth (the
/// distance from the camera, infinite where nothing was hit), normal and
/// position in Tequila's Y-up world, albedo, the two Cryptomattes, and each
/// light group's image, in the rendering space.
struct CyclesPasses {
    /// Tequila's light-path passes, as Cycles' own light passes make them
    /// ([`CYCLES_LIGHT_PASSES`]).
    radiance: Vec<(String, Vec<tequila_core::spectrum::Spectrum>)>,
    clown: Vec<tequila_core::spectrum::Spectrum>,
    depth: Vec<f32>,
    normal: Vec<tequila_core::math::Vec3>,
    position: Vec<tequila_core::math::Vec3>,
    albedo: Vec<tequila_core::spectrum::Spectrum>,
    mattes: Vec<tequila_render_core::cryptomatte::Cryptomatte>,
    groups: Vec<(String, Vec<tequila_core::spectrum::Spectrum>)>,
}

/// Tequila's light-path passes (`aov::standard_passes`) from Cycles' own:
/// each a sum, over Cycles' diffuse, glossy and transmission, of its direct
/// light, its indirect light or both, by that closure's colour, as a
/// compositor puts Blender's passes back together. Caustics has none.
const CYCLES_LIGHT_PASSES: [(&str, &[(&str, bool, bool)]); 5] = [
    ("Diffuse", &[("diffuse", true, true)]),
    ("Lighting", &[("diffuse", true, false), ("glossy", true, false)]),
    ("GlobalIllumination", &[("diffuse", false, true), ("glossy", false, true)]),
    ("Reflection", &[("glossy", true, true)]),
    ("Refraction", &[("transmission", true, true)]),
];

/// The passes only Tequila's renderer writes, which a Cycles render goes
/// without.
const TEQUILA_ONLY_PASSES: &str = "Caustics, AmbientOcclusion, Shadow and Labels";

/// Cryptomatte's layers of each kind: two ranks a layer, the six Tequila's
/// renderer keeps.
const CRYPTOMATTE_LAYERS: usize = tequila_render_core::cryptomatte::RANKS.div_ceil(2);

/// What Cycles' passes come back as, named as Tequila's renderer names
/// them. Read only for a render that asked for passes or light groups.
fn cycles_passes(
    session: &mut tequila_render_cycles::Session,
    job: &BatchJob,
    groups: &[String],
    want_passes: bool,
) -> Result<CyclesPasses, String> {
    use tequila_core::spectrum::Spectrum;
    use tequila_render_cycles::translate::{from_cycles, to_rendering};
    let settings = &job.settings;
    let (width, height) = (settings.width, settings.height);
    let pixels = width * height;
    let read = |session: &mut tequila_render_cycles::Session, name: &str, channels: usize| session.render_pass(name, channels, width, height);
    let depth_z = read(session, "Depth", 1)?;
    let position: Vec<_> = read(session, "Position", 3)?.chunks_exact(3).map(|p| from_cycles([p[0], p[1], p[2]])).collect();
    let normal = read(session, "Normal", 3)?.chunks_exact(3).map(|n| from_cycles([n[0], n[1], n[2]])).collect();
    let albedo = read(session, "Albedo", 3)?.chunks_exact(3).map(|a| to_rendering(a[0], a[1], a[2]).sanitized()).collect();
    let eye = job.camera.position();
    let depth = (0..pixels).map(|i| if depth_z[i] > 0.0 && depth_z[i] < 1e9 { (position[i] - eye).length() } else { f32::INFINITY }).collect();
    let mut radiance = Vec::new();
    let mut clown = Vec::new();
    let mut mattes = Vec::new();
    if want_passes {
        let mut light = std::collections::HashMap::new();
        for kind in ["diffuse", "glossy", "transmission"] {
            for part in ["direct", "indirect", "color"] {
                let name = format!("{kind}_{part}");
                let image: Vec<Spectrum> = read(session, &name, 3)?.chunks_exact(3).map(|c| to_rendering(c[0], c[1], c[2]).sanitized()).collect();
                light.insert(name, image);
            }
        }
        for (name, terms) in CYCLES_LIGHT_PASSES {
            let image = (0..pixels)
                .map(|i| {
                    terms.iter().fold(Spectrum::ZERO, |sum, &(kind, direct, indirect)| {
                        let lit = |part: &str| light[&format!("{kind}_{part}")][i];
                        let incoming = match (direct, indirect) {
                            (true, true) => lit("direct") + lit("indirect"),
                            (true, false) => lit("direct"),
                            _ => lit("indirect"),
                        };
                        sum + incoming * lit("color")
                    })
                })
                .collect();
            radiance.push((name.to_string(), image));
        }
        // The object's id, one more than its instance's (`pass_id`).
        clown = read(session, "Object", 1)?
            .into_iter()
            .map(|id| if id >= 0.5 { tequila_render_core::aov::clown_color(id.round() as u32 - 1) } else { Spectrum::ZERO })
            .collect();
        for (kind, names) in [("CryptoObject", &job.object_names), ("CryptoMaterial", &job.material_names)] {
            let mut matte = tequila_render_core::cryptomatte::Cryptomatte::new(kind, width, height);
            for name in names.iter() {
                matte.register(name);
            }
            for layer in 0..CRYPTOMATTE_LAYERS {
                let ranks = read(session, &format!("{kind}{layer:02}"), 4)?;
                for (i, rank) in ranks.chunks_exact(4).enumerate() {
                    for (id, coverage) in [(rank[0], rank[1]), (rank[2], rank[3])] {
                        if coverage > 0.0 {
                            matte.accumulate(i % width, i / width, id, coverage);
                        }
                    }
                }
            }
            matte.finish(1);
            mattes.push(matte);
        }
    }
    let mut kept = Vec::with_capacity(groups.len());
    for name in groups {
        let image: Vec<Spectrum> = read(session, name, 3)?.chunks_exact(3).map(|c| to_rendering(c[0], c[1], c[2]).sanitized()).collect();
        kept.push((name.clone(), image));
    }
    Ok(CyclesPasses { radiance, clown, depth, normal, position, albedo, mattes, groups: kept })
}

/// Render with Cycles on `engine`. An error here is Cycles' or the
/// device's; [`finish_cycles`] does the rest.
fn render_cycles(job: &BatchJob, engine: RenderEngine, events: &Sender<BatchEvent>, cancel: &AtomicBool) -> Result<CyclesFrame, String> {
    use tequila_render_cycles::{translate, Session};
    let settings = &job.settings;
    let started = Instant::now();

    let mut session = Session::new(engine.cycles_device(), 0, None)
        .map_err(|e| format!("Cycles could not start on {}: {e}", engine.label()))?;
    // Passes and light groups, named as Tequila's renderer names them.
    let want_passes = settings.effective_write_passes();
    let keeping = want_passes || settings.relight;
    let groups: Vec<String> = if keeping {
        tequila_render_core::aov::AovSet::light_groups_for(&job.scene).light_paths.into_iter().map(|pass| pass.name).collect()
    } else {
        Vec::new()
    };
    let view = translate::View {
        camera: job.camera,
        width: settings.width,
        height: settings.height,
        transparent: settings.alpha_mode.transparent_background(),
        ground_illumination: job.config.ground_illumination,
        light_materials: job.config.light_source_materials,
        light_groups: keeping,
    };
    let translation = if keeping {
        let names = translate::Names { objects: job.object_names.clone(), materials: job.material_names.clone() };
        translate::build_named(&mut session, &job.scene, &view, &names)?
    } else {
        translate::build(&mut session, &job.scene, &view)?
    };
    if keeping {
        session.add_pass("Depth", "depth", 1, None)?;
        session.add_pass("Position", "position", 3, None)?;
        session.add_pass("Normal", "normal", 3, None)?;
        session.add_pass("Albedo", "denoising_albedo", 3, None)?;
        if want_passes {
            session.cryptomatte(CRYPTOMATTE_LAYERS)?;
            session.add_pass("Object", "object_id", 1, None)?;
            for kind in ["diffuse", "glossy", "transmission"] {
                for part in ["direct", "indirect", "color"] {
                    let name = format!("{kind}_{part}");
                    session.add_pass(&name, &name, 3, None)?;
                }
            }
        }
        // A light group is a combined pass kept to its group, as Blender
        // makes them.
        for name in &groups {
            session.add_pass(name, "combined", 3, Some(name))?;
        }
    }
    for warning in &translation.warnings {
        eprintln!("Cycles: {warning}");
    }
    let under = translation
        .background_under
        .then(|| translate::background(&job.scene, &job.camera, settings.width, settings.height));
    // Kept passes are Cycles' noisy ones. Its denoiser, which denoises every
    // pass it can and drops their light groups, is left out, and the
    // picture is denoised below against the passes' guides as the groups
    // are.
    translate::configure(
        &mut session,
        &translate::Quality::with_lighting(settings.adaptive, settings.denoise && !keeping, settings.max_seconds.map(f64::from), &job.config)
            .with_clamping(settings.reject_fireflies, settings.clamp_indirect, settings.prefilter_guides)
            .with_render_controls(settings.clamp_direct, settings.pixel_filter, settings.adaptive_threshold),
    )?;

    let _ = events.send(BatchEvent::Stage { id: job.id, label: "Rendering" });
    let total = settings.max_samples;
    let mut last = Instant::now();
    // The most samples Cycles said it had reached: it clears its own count
    // as the render winds up.
    let mut reached = 0;
    let rgba = session.render_until(
        settings.width,
        settings.height,
        total,
        |_, samples| {
            reached = reached.max(samples);
            // Cycles reports often; the queue only needs to hear a few times a
            // second.
            if last.elapsed().as_secs_f32() >= 0.25 {
                last = Instant::now();
                let _ = events.send(BatchEvent::Progress {
                    id: job.id,
                    samples,
                    seconds: started.elapsed().as_secs_f64(),
                });
            }
        },
        Some(cancel),
    )?;
    let stopped = cancel.load(Ordering::Acquire);
    let seconds = started.elapsed().as_secs_f64();
    let halt = if stopped || settings.max_seconds.is_some_and(|limit| seconds >= f64::from(limit)) {
        Halt::Time
    } else {
        Halt::Samples
    };
    // What it reached, which a time limit or a cancel leaves short of what
    // was asked for.
    let rendered = if halt == Halt::Samples { total } else { reached.min(total) };
    let passes = if keeping { Some(cycles_passes(&mut session, job, &groups, want_passes)?) } else { None };
    let denoise_after = passes.as_ref().filter(|_| settings.denoise).map(|passes| {
        (passes.albedo.iter().flat_map(|s| s.to_array()).collect::<Vec<f32>>(), passes.normal.iter().flat_map(|n| [n.x, n.y, n.z]).collect::<Vec<f32>>())
    });

    // Cycles' film is premultiplied scene-linear Rec.709, which is what the
    // output takes, once back in the rendering space.
    let mut color = Vec::with_capacity(settings.width * settings.height);
    let mut alpha = Vec::with_capacity(settings.width * settings.height);
    for (i, px) in rgba.chunks_exact(4).enumerate() {
        color.push(match &under {
            Some(background) => translate::lay_under(px, background[i]),
            None => translate::to_rendering(px[0], px[1], px[2]).sanitized(),
        });
        alpha.push(if view.transparent { px[3].clamp(0.0, 1.0) } else { 1.0 });
    }
    if let Some((albedo, normal)) = denoise_after {
        let _ = events.send(BatchEvent::Stage { id: job.id, label: "Denoising" });
        if let Ok(denoiser) = tequila_denoise::Denoiser::shared() {
            let flat: Vec<f32> = color.iter().flat_map(|s| s.to_array()).collect();
            let quality = tequila_denoise::DenoiseSettings {
                quality: tequila_denoise::Quality::High,
                strength: settings.denoise_strength,
                prefilter_guides: settings.prefilter_guides,
            };
            if let Ok(clean) = denoiser.denoise(settings.width, settings.height, &flat, &albedo, &normal, quality) {
                color = clean
                    .data
                    .chunks(3)
                    .map(|c| tequila_core::spectrum::Spectrum::from_rendering_rgb(tequila_core::math::Vec3 { x: c[0], y: c[1], z: c[2] }))
                    .collect();
            }
        }
    }
    Ok(CyclesFrame {
        color,
        alpha,
        passes,
        transparent: view.transparent,
        floor_reflection: translation.floor_reflection,
        started,
        seconds,
        rendered,
        halt,
        stopped,
    })
}

/// Lay Tequila's floor reflection and cut-out over what Cycles rendered,
/// and save it.
fn finish_cycles(job: &BatchJob, frame: CyclesFrame, events: &Sender<BatchEvent>, cancel: &AtomicBool) -> Result<BatchEvent, String> {
    let settings = &job.settings;
    let CyclesFrame { mut color, mut alpha, passes, transparent, floor_reflection, started, seconds, rendered, halt, stopped } = frame;
    let _ = events.send(BatchEvent::Stage { id: job.id, label: "Saving image" });
    // The floor's reflection, which Cycles was left without: Tequila's,
    // through the same camera, laid over (`tequila_render_cpu::floor`).
    if floor_reflection {
        let _ = events.send(BatchEvent::Stage { id: job.id, label: "Floor reflection" });
        let device = tequila_render_cpu::Device::shared().map_err(|e| e.to_string())?;
        let intersector = tequila_render_cpu::EmbreeIntersector::build(&device, (*job.scene).clone()).map_err(|e| e.to_string())?;
        let mut layer = Framebuffer::new(settings.width, settings.height);
        // Half the image's samples, which a reflection's share of the pixel
        // hides the rest of the noise of, and the denoiser the remainder;
        // fewer once the render is stopped or out of time, as Cycles was.
        let samples = (settings.max_samples / 2).clamp(16, 512);
        let over = || {
            cancel.load(Ordering::Acquire) || settings.max_seconds.is_some_and(|limit| started.elapsed().as_secs_f64() >= f64::from(limit))
        };
        tequila_render_cpu::floor::render_floor_reflection_until(&intersector, &mut layer, job.camera, &job.config, samples, &over);
        let guides = layer.to_denoiser_buffers();
        let denoised = settings
            .denoise
            .then(|| tequila_denoise::Denoiser::shared().ok())
            .flatten()
            .and_then(|denoiser| {
                denoiser
                    .denoise(
                        settings.width,
                        settings.height,
                        &guides.color,
                        &guides.albedo,
                        &guides.normal,
                        tequila_denoise::DenoiseSettings { quality: tequila_denoise::Quality::High, strength: 1.0, prefilter_guides: true },
                    )
                    .ok()
            });
        let reflected: &[f32] = denoised.as_ref().map_or(&guides.color, |image| &image.data);
        // How much each pixel's reflection takes of what is under it,
        // denoised as its colour is. Left as sampled, a blurred
        // reflection's fade put dark specks on a light backdrop and grain in
        // a transparent one.
        let shares = smooth_alpha(settings.width, settings.height, layer.pixels().iter().map(|p| p.alpha()).collect(), settings.denoise);
        let bright = tequila_render_cpu::floor::alpha_by_brightness(&job.scene);
        for (i, &share) in shares.iter().enumerate() {
            if share > 0.0 {
                let reflection = tequila_core::spectrum::Spectrum::from_rendering_rgb(tequila_core::math::vec3(
                    reflected[3 * i].max(0.0),
                    reflected[3 * i + 1].max(0.0),
                    reflected[3 * i + 2].max(0.0),
                ));
                let (reflection, share) =
                    if bright { tequila_render_cpu::floor::by_brightness(reflection, share, job.style.exposure) } else { (reflection, share) };
                (color[i], alpha[i]) = tequila_render_cpu::floor::lay_reflection(color[i], alpha[i], reflection, share);
            }
        }
    }
    // Cycles denoises its colour and leaves its alpha as sampled, so a
    // transparent render's shadow came out grainy: random dots once laid
    // over a page.
    let alpha = if transparent { smooth_alpha(settings.width, settings.height, alpha, settings.denoise) } else { alpha };
    // Cut out along the outline, which Cycles has no film setting for: the
    // same camera rays cast against the same scene in Tequila's own tracer.
    let (color, alpha) = if settings.alpha_mode == AlphaMode::Silhouette {
        let _ = events.send(BatchEvent::Stage { id: job.id, label: "Cutting out" });
        let device = tequila_render_cpu::Device::shared().map_err(|e| e.to_string())?;
        let intersector = tequila_render_cpu::EmbreeIntersector::build(&device, (*job.scene).clone()).map_err(|e| e.to_string())?;
        tequila_render_cpu::Silhouette::trace(&intersector, &job.camera, settings.width, settings.height, SILHOUETTE_SAMPLES, 0).cut(&color)
    } else {
        (color, alpha)
    };

    let directory = if settings.output_directory.trim().is_empty() {
        crate::render_window::fallback_directory()
    } else {
        std::path::PathBuf::from(settings.output_directory.trim())
    };
    std::fs::create_dir_all(&directory).map_err(|e| format!("{} could not be made: {e}", directory.display()))?;
    let mut fields = job.fields.clone();
    fields.width = settings.width;
    fields.height = settings.height;
    fields.samples = rendered;
    let format = crate::render_window::image_format(settings);
    let path = tequila_format::output::first_free(tequila_format::output::output_path(
        &directory,
        &settings.filename_template,
        &fields,
        format,
    ));
    // The light groups, with what Tequila laid over Cycles' picture - the
    // floor's reflection, the backdrop under it - as one more, so the
    // groups always add up to the picture saved.
    let groups = passes.as_ref().map(|passes| {
        let mut groups = passes.groups.clone();
        let rest: Vec<tequila_core::spectrum::Spectrum> = (0..color.len())
            .map(|i| groups.iter().fold(color[i], |left, (_, image)| left - image[i]))
            .collect();
        if rest.iter().any(|c| c.luminance().abs() > 1e-3) {
            groups.push(("Other".to_string(), rest));
        }
        groups
    });
    let want_passes = settings.effective_write_passes();
    let output = RenderOutput {
        width: settings.width,
        height: settings.height,
        color,
        alpha,
        // Its light passes and light groups, as Tequila's renderer lays a
        // layered file's out.
        layers: match (&passes, &groups, want_passes) {
            (Some(passes), Some(groups), true) => passes.radiance.iter().chain(groups.iter()).map(|(name, image)| (name.clone(), image.clone(), None)).collect(),
            _ => Vec::new(),
        },
        style: job.style,
        display: job.display.clone(),
        alpha_mode: output_alpha(settings),
    };
    let preview = output.to_rgba8();
    match (format, &passes) {
        (tequila_format::output::ImageFormat::Exr(precision), Some(passes)) if want_passes => {
            let (color, alpha) = output.matted();
            let data = tequila_format::passes::DataLayers {
                depth: Some(&passes.depth),
                normal: Some(&passes.normal),
                position: Some(&passes.position),
                albedo: Some(&passes.albedo),
                clown: Some(&passes.clown),
            };
            let named: Vec<(String, Vec<tequila_core::spectrum::Spectrum>)> = passes.radiance.iter().cloned().chain(groups.clone().unwrap_or_default()).collect();
            let mattes: Vec<&tequila_render_core::cryptomatte::Cryptomatte> = passes.mattes.iter().collect();
            tequila_format::passes::exr_from_layers(settings.width, settings.height, color, alpha, precision, &named, &data, &mattes)
                .write(&path)
                .map_err(|e| e.to_string())?;
        }
        _ => output.write(&path, format).map_err(|e| e.to_string())?,
    }
    write_sidecars(&path, job, settings, rendered, seconds, halt)?;
    let relight = match (passes, groups, settings.relight) {
        (Some(passes), Some(groups), true) => {
            let _ = events.send(BatchEvent::Stage { id: job.id, label: "Keeping light groups" });
            Some(Box::new(Relight::from_groups(groups, &passes.albedo, &passes.normal, settings, &output, path.clone(), format)))
        }
        _ => None,
    };

    Ok(BatchEvent::Finished {
        id: job.id,
        path: path.to_string_lossy().to_string(),
        width: settings.width,
        height: settings.height,
        samples: rendered,
        seconds,
        preview,
        halt,
        stopped,
        relight,
    })
}

/// A finished render split by light group (§5.5), to relight without
/// rendering again: each group's image, already denoised, and everything
/// needed to develop and save a recombination.
pub struct Relight {
    pub width: usize,
    pub height: usize,
    /// The group's name and its image, scene-linear.
    pub groups: Vec<(String, Vec<tequila_core::spectrum::Spectrum>)>,
    pub alpha: Vec<f32>,
    pub style: ImageStyle,
    pub display: DisplayPipeline,
    pub alpha_mode: OutputAlphaMode,
    pub format: tequila_format::output::ImageFormat,
    /// The render it came from, which a relit copy is saved beside.
    pub path: std::path::PathBuf,
}

impl Relight {
    fn gather(
        framebuffer: &Framebuffer,
        buffers: &AovBuffers,
        set: &AovSet,
        samples: u32,
        settings: &RenderSettings,
        output: &RenderOutput,
        path: std::path::PathBuf,
        format: tequila_format::output::ImageFormat,
    ) -> Relight {
        let (width, height) = (settings.width, settings.height);
        let denoiser = settings.denoise.then(|| tequila_denoise::Denoiser::shared().ok()).flatten();
        let guides = framebuffer.to_denoiser_buffers();
        let groups = set
            .light_paths
            .iter()
            .enumerate()
            .filter(|(_, pass)| AovSet::is_light_group(&pass.name))
            .map(|(index, pass)| {
                let mut image: Vec<tequila_core::spectrum::Spectrum> = (0..height)
                    .flat_map(|y| (0..width).map(move |x| (x, y)))
                    .map(|(x, y)| buffers.passes.mean(index, x, y, samples))
                    .collect();
                // Each group denoised on its own, against the same guides as
                // the image, so a recombination is as clean as the render.
                if let Some(denoiser) = &denoiser {
                    let flat: Vec<f32> = image.iter().flat_map(|s| s.to_array()).collect();
                    if let Ok(clean) = denoiser.denoise(
                        width,
                        height,
                        &flat,
                        &guides.albedo,
                        &guides.normal,
                        tequila_denoise::DenoiseSettings {
                            quality: tequila_denoise::Quality::High,
                            strength: settings.denoise_strength,
                            prefilter_guides: settings.prefilter_guides,
                        },
                    ) {
                        image = clean
                            .data
                            .chunks(3)
                            .map(|c| {
                                tequila_core::spectrum::Spectrum::from_rendering_rgb(tequila_core::math::Vec3 {
                                    x: c[0],
                                    y: c[1],
                                    z: c[2],
                                })
                            })
                            .collect();
                    }
                }
                (pass.name.clone(), image)
            })
            .collect();
        Relight {
            width,
            height,
            groups,
            alpha: output.alpha.clone(),
            style: output.style,
            display: output.display.clone(),
            alpha_mode: output.alpha_mode,
            format,
            path,
        }
    }

    /// The same from groups another renderer made (Cycles'), denoised each
    /// against `albedo` and `normal` as the image is - all but `Other`, what
    /// Tequila laid over that renderer's picture, which is exact as it is.
    fn from_groups(
        groups: Vec<(String, Vec<tequila_core::spectrum::Spectrum>)>,
        albedo: &[tequila_core::spectrum::Spectrum],
        normal: &[tequila_core::math::Vec3],
        settings: &RenderSettings,
        output: &RenderOutput,
        path: std::path::PathBuf,
        format: tequila_format::output::ImageFormat,
    ) -> Relight {
        let (width, height) = (settings.width, settings.height);
        let denoiser = settings.denoise.then(|| tequila_denoise::Denoiser::shared().ok()).flatten();
        let albedo: Vec<f32> = albedo.iter().flat_map(|s| s.to_array()).collect();
        let normal: Vec<f32> = normal.iter().flat_map(|n| [n.x, n.y, n.z]).collect();
        let groups = groups
            .into_iter()
            .map(|(name, mut image)| {
                if let (Some(denoiser), false) = (&denoiser, name == "Other") {
                    let flat: Vec<f32> = image.iter().flat_map(|s| s.to_array()).collect();
                    let quality = tequila_denoise::DenoiseSettings {
                        quality: tequila_denoise::Quality::High,
                        strength: settings.denoise_strength,
                        prefilter_guides: settings.prefilter_guides,
                    };
                    if let Ok(clean) = denoiser.denoise(width, height, &flat, &albedo, &normal, quality) {
                        image = clean
                            .data
                            .chunks(3)
                            .map(|c| tequila_core::spectrum::Spectrum::from_rendering_rgb(tequila_core::math::Vec3 { x: c[0], y: c[1], z: c[2] }))
                            .collect();
                    }
                }
                (name, image)
            })
            .collect();
        Relight {
            width,
            height,
            groups,
            alpha: output.alpha.clone(),
            style: output.style,
            display: output.display.clone(),
            alpha_mode: output.alpha_mode,
            format,
            path,
        }
    }

    /// The groups recombined, each scaled by its gain: at full size, or no
    /// wider than `max_width` for a preview.
    pub fn compose(&self, gains: &[tequila_core::spectrum::Spectrum], max_width: usize) -> RenderOutput {
        let width = self.width.min(max_width.max(1));
        let height = (self.height * width / self.width.max(1)).max(1);
        let mut color = Vec::with_capacity(width * height);
        let mut alpha = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let i = (y * self.height / height) * self.width + x * self.width / width;
                let mut sum = tequila_core::spectrum::Spectrum::ZERO;
                for ((_, image), gain) in self.groups.iter().zip(gains) {
                    sum = sum + image[i] * *gain;
                }
                color.push(sum);
                alpha.push(self.alpha[i]);
            }
        }
        RenderOutput {
            width,
            height,
            color,
            alpha,
            layers: Vec::new(),
            style: self.style,
            display: self.display.clone(),
            alpha_mode: self.alpha_mode,
        }
    }

    /// Save a recombination at full size beside the render, as
    /// `<render>_relit`, never over anything.
    pub fn save(&self, gains: &[tequila_core::spectrum::Spectrum]) -> Result<std::path::PathBuf, String> {
        let target = tequila_format::output::first_free(layer_path(&self.path, "relit"));
        self.compose(gains, usize::MAX)
            .write(&target, self.format)
            .map_err(|e| e.to_string())?;
        Ok(target)
    }
}

/// A layer's file: the main image's name with the layer's after it.
fn layer_path(main: &std::path::Path, layer: &str) -> std::path::PathBuf {
    let clean: String = layer
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let stem = main.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match main.extension() {
        Some(ext) => format!("{stem}_{clean}.{}", ext.to_string_lossy()),
        None => format!("{stem}_{clean}"),
    };
    main.with_file_name(name)
}

/// Rays a pixel for the Silhouette mode's outline: enough for a smooth
/// edge, and a small fraction of a render's cost, since nothing is shaded.
const SILHOUETTE_SAMPLES: u32 = 24;

pub(crate) fn output_alpha(settings: &RenderSettings) -> OutputAlphaMode {
    match settings.alpha_mode {
        AlphaMode::None => OutputAlphaMode::None,
        AlphaMode::Straight => OutputAlphaMode::Straight,
        AlphaMode::Premultiplied => OutputAlphaMode::Premultiplied,
        AlphaMode::Aliased => OutputAlphaMode::Aliased,
        // Cut colour is premultiplied; straight is what an editor opening
        // a cut-out product shot expects.
        AlphaMode::Silhouette => OutputAlphaMode::Straight,
    }
}

/// The colour and coverage a file gets from an accumulation buffer:
/// denoised when asked for, and never into the buffer itself. With
/// `foreground`, for a file with alpha, the colour leaves the background out
/// ([`tequila_render_core::framebuffer::Pixel::mean_foreground`]).
fn develop(
    framebuffer: &Framebuffer,
    settings: &RenderSettings,
    foreground: bool,
) -> (Vec<tequila_core::spectrum::Spectrum>, Vec<f32>) {
    // Denoised for the file, never into the accumulation buffer. §13: "applied
    // for display and for output, never into the accumulation buffer" —
    // accumulating a denoised result would poison convergence, and the passes
    // written beside this image come from the buffer that was not touched.
    let denoised = settings
        .denoise
        .then(|| tequila_denoise::Denoiser::shared().ok())
        .flatten()
        .and_then(|denoiser| {
            let buffers = framebuffer.to_denoiser_buffers_with(foreground);
            denoiser
                .denoise(
                    settings.width,
                    settings.height,
                    &buffers.color,
                    &buffers.albedo,
                    &buffers.normal,
                    tequila_denoise::DenoiseSettings {
                        // A finished render is worth the slower filter: it runs
                        // once, against a render that took minutes.
                        quality: tequila_denoise::Quality::High,
                        strength: settings.denoise_strength,
                        prefilter_guides: settings.prefilter_guides,
                    },
                )
                .ok()
        });

    let mut color = Vec::with_capacity(settings.width * settings.height);
    let mut alpha = Vec::with_capacity(settings.width * settings.height);
    for y in 0..settings.height {
        for x in 0..settings.width {
            let pixel = framebuffer.pixel(x, y);
            color.push(match &denoised {
                Some(image) => {
                    let i = (y * settings.width + x) * 3;
                    tequila_core::spectrum::Spectrum::from_rendering_rgb(
                        tequila_core::math::Vec3 {
                            x: image.data[i],
                            y: image.data[i + 1],
                            z: image.data[i + 2],
                        },
                    )
                }
                None if foreground => pixel.mean_foreground(),
                None => pixel.mean(),
            });
            alpha.push(pixel.alpha());
        }
    }
    let alpha = if foreground { smooth_alpha(settings.width, settings.height, alpha, settings.denoise) } else { alpha };
    (color, alpha)
}

/// An image's alpha, denoised when the render is (`Denoiser::denoise_alpha`):
/// the colour's denoising leaves it as sampled, so a transparent floor's
/// shadow and reflection were grainy. As it was if the denoiser is missing.
pub(crate) fn smooth_alpha(width: usize, height: usize, alpha: Vec<f32>, denoise: bool) -> Vec<f32> {
    if !denoise {
        return alpha;
    }
    match tequila_denoise::Denoiser::shared().and_then(|d| d.denoise_alpha(width, height, &alpha)) {
        Ok(smooth) => smooth,
        Err(_) => alpha,
    }
}

/// The passes, as layers for a layered format.
fn layers_of(
    aovs: Option<&AovBuffers>,
    set: &AovSet,
    samples: u32,
    settings: &RenderSettings,
) -> Vec<(String, Vec<tequila_core::spectrum::Spectrum>, Option<Vec<f32>>)> {
    use tequila_core::spectrum::Spectrum;
    let Some(aovs) = aovs else {
        return Vec::new();
    };
    let (width, height) = (settings.width, settings.height);
    let each = |f: &dyn Fn(usize, usize) -> Spectrum| {
        let mut pixels = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                pixels.push(f(x, y));
            }
        }
        pixels
    };
    let mut layers: Vec<(String, Vec<Spectrum>, Option<Vec<f32>>)> = set
        .light_paths
        .iter()
        .enumerate()
        .filter(|(_, pass)| pass.name != "Beauty")
        .map(|(index, pass)| (pass.name.clone(), each(&|x, y| aovs.passes.mean(index, x, y, samples)), None))
        .collect();

    // The data passes a compositor reaches for, which a PSD had none of:
    // occlusion and shadow to darken with, and the labels alone, with their
    // own transparency, to lay over the picture or replace.
    if set.data.ambient_occlusion {
        layers.push((
            "AmbientOcclusion".into(),
            each(&|x, y| Spectrum::splat(aovs.mean_occlusion(x, y, samples))),
            None,
        ));
    }
    if set.data.shadow {
        layers.push(("Shadow".into(), each(&|x, y| Spectrum::splat(aovs.shadow(x, y))), None));
    }
    if set.data.labels {
        // Straight colour, as a layer's pixels are, with the coverage beside it.
        let straight = each(&|x, y| {
            let (colour, alpha) = aovs.mean_labels(x, y, samples);
            if alpha > 1e-4 { colour * (1.0 / alpha) } else { Spectrum::ZERO }
        });
        let mut coverage = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                coverage.push(aovs.mean_labels(x, y, samples).1.clamp(0.0, 1.0));
            }
        }
        layers.push(("Labels".into(), straight, Some(coverage)));
    }
    layers
}

fn write_sidecars(
    path: &std::path::Path,
    job: &BatchJob,
    settings: &RenderSettings,
    samples: u32,
    seconds: f64,
    halt: Halt,
) -> Result<(), String> {
    if !settings.write_xmp && !settings.write_text_metadata {
        return Ok(());
    }
    let mut metadata = Metadata::default();
    metadata
        .set("Renderer", format!("Tequila {}", env!("CARGO_PKG_VERSION")))
        .set("Job", job.name.clone())
        .set("Camera", job.fields.camera.clone())
        .set(
            "Resolution",
            format!("{}x{}", settings.width, settings.height),
        )
        .set("Samples", samples.to_string())
        .set("Seconds", format!("{seconds:.2}"))
        .set(
            "StoppedBecause",
            match halt {
                Halt::Samples => "the sample limit was reached",
                Halt::Time => "the time limit was reached",
                Halt::Converged => "every pixel converged",
            },
        )
        .set("AlphaMode", settings.alpha_mode.label())
        // Bias the user should know about, rather than has to discover. A
        // render with clamping on is not the same render as one without.
        .set(
            "FireflyRejection",
            if settings.reject_fireflies { "on" } else { "off" },
        );
    if let Some(clamp) = settings.clamp_indirect {
        metadata.set("SampleClamp", format!("{clamp}"));
    }

    if settings.write_text_metadata {
        let text = path.with_extension("txt");
        std::fs::write(&text, metadata.to_text())
            .map_err(|e| format!("{} could not be written: {e}", text.display()))?;
    }
    if settings.write_xmp {
        let xmp = path.with_extension("xmp");
        std::fs::write(&xmp, metadata.to_xmp())
            .map_err(|e| format!("{} could not be written: {e}", xmp.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec3, Vec3};
    use tequila_scene::render::OutputFormat;
    use std::time::Duration;

    #[test]
    fn cropped_render_and_layers_reach_the_sample_limit() {
        let directory = scratch("tequila-audit-crop");
        let renderer = BatchRenderer::new();
        let mut render = job(1, &directory, RenderSettings {
            width: 32, height: 24, max_samples: 4, max_seconds: Some(2),
            denoise: false, adaptive: false,
            region: Some(tequila_scene::render::Region { x: 8, y: 6, width: 8, height: 6 }),
            ..Default::default()
        });
        render.layers.push(("crop".into(), render.scene.clone()));
        renderer.submit(render);
        match wait_for_finish(&renderer) {
            BatchEvent::Finished { samples, halt, path, .. } => {
                assert_eq!(samples, 4);
                assert_eq!(halt, Halt::Samples);
                assert!(layer_path(std::path::Path::new(&path), "crop").is_file());
            }
            BatchEvent::Failed { message, .. } => panic!("{message}"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn empty_region_reports_failure_without_killing_worker() {
        let directory = scratch("tequila-audit-empty-crop");
        let renderer = BatchRenderer::new();
        renderer.submit(job(1, &directory, RenderSettings {
            width: 16, height: 16, region: Some(tequila_scene::render::Region { x: 20, y: 0, width: 4, height: 4 }),
            ..Default::default()
        }));
        assert!(matches!(wait_for_finish(&renderer), BatchEvent::Failed { .. }));
        renderer.submit(job(2, &directory, RenderSettings { width: 16, height: 16, max_samples: 2, denoise: false, ..Default::default() }));
        assert!(matches!(wait_for_finish(&renderer), BatchEvent::Finished { id: 2, .. }));
    }

    /// With the Cycles engine a queued render goes through Cycles and comes
    /// back like any other: the image written, at its samples, and lit.
    /// Skipped where Cycles is not installed (`TEQUILA_CYCLES`).
    #[test]
    fn a_cycles_render_writes_its_image() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let directory = scratch("tequila-cycles-batch");
        let renderer = BatchRenderer::new();
        renderer.submit(job(1, &directory, RenderSettings {
            engine: tequila_scene::render::RenderEngine::CyclesCpu,
            width: 64, height: 48, max_samples: 8, denoise: false,
            ..Default::default()
        }));
        match wait_for_finish(&renderer) {
            BatchEvent::Finished { path, samples, preview, .. } => {
                assert!(std::path::Path::new(&path).is_file(), "{path} was not written");
                assert_eq!(samples, 8);
                let mean = preview.iter().step_by(4).map(|&v| f32::from(v)).sum::<f32>() / (preview.len() / 4) as f32;
                assert!(mean > 20.0, "the render came back black: mean red {mean}");
            }
            BatchEvent::Failed { message, .. } => panic!("{message}"),
            _ => unreachable!(),
        }
    }

    /// Tequila's lights come out of Cycles as bright as from Tequila's own
    /// renderer: a point light and an area light, each alone in the dark,
    /// light the demo scene to within a few percent of each other. The two
    /// state power differently (candela and nits against Cycles' strength),
    /// so this is the conversion's test. Skipped where Cycles is not
    /// installed.
    #[test]
    fn cycles_lights_match_tequila_lights() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        use tequila_render_core::light::{AreaLight, Light, LightKind, LightSet, PointLight};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let lamps = [
            Light::new(LightKind::Point(PointLight { position: vec3(1.0, 2.0, 1.5), intensity: Spectrum::splat(4.0), radius: 0.05 })),
            Light::new(LightKind::Area(AreaLight {
                // Above and beside the sphere (radius 1, resting on the
                // floor, so its top is at 2): clear of it, lighting both it
                // and the floor.
                position: vec3(1.5, 4.0, 1.5),
                // Facing down: cross(u, v) is -Y.
                half_u: vec3(0.4, 0.0, 0.0),
                half_v: vec3(0.0, 0.0, 0.3),
                radiance: Spectrum::splat(40.0),
                disc: false,
                two_sided: false,
                spread: std::f32::consts::PI,
            })),
        ];
        for (i, lamp) in lamps.into_iter().enumerate() {
            let mut means = Vec::new();
            for engine in [tequila_scene::render::RenderEngine::Tequila, tequila_scene::render::RenderEngine::CyclesCpu] {
                let directory = scratch(&format!("tequila-cycles-lights-{i}-{}", engine.key()));
                let renderer = BatchRenderer::new();
                let mut render = job(1, &directory, RenderSettings {
                    engine,
                    width: 64, height: 48, max_samples: 256, denoise: false,
                    ..Default::default()
                });
                let mut scene = (*render.scene).clone().with_lights(Arc::new(LightSet::new(vec![lamp.clone()])));
                scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
                scene.background = Background::Color(Spectrum::ZERO);
                render.scene = Arc::new(scene);
                renderer.submit(render);
                match wait_for_finish(&renderer) {
                    BatchEvent::Finished { preview, .. } => {
                        let mean = preview.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>()
                            / (preview.len() / 4 * 3) as f64;
                        means.push(mean);
                    }
                    BatchEvent::Failed { message, .. } => panic!("{message}"),
                    _ => unreachable!(),
                }
            }
            let (tequila, cycles) = (means[0], means[1]);
            assert!(tequila > 5.0, "light {i} left the Tequila render black");
            assert!(
                (cycles / tequila - 1.0).abs() < 0.1,
                "light {i}: Tequila's render averages {tequila:.1}, Cycles' {cycles:.1}"
            );
        }
    }

    /// An area light node shines along its local +Z, the cross of its width
    /// and its height, in both engines, and still does with a photometric
    /// file. The board's panel is turned so its +Z points down at the board;
    /// turned half a turn more, so its -Z does, as a lamp placed with a
    /// Blender matrix is, it lights nothing. The docs said -Z, and a scene
    /// built from them came out black; and a file aimed every light along
    /// -Z, so ticking "Photometric file" turned the panel round, though the
    /// Lighting tab says the shape still decides which way it points. Cycles
    /// is skipped where it is not installed.
    #[test]
    fn an_area_light_shines_along_its_local_plus_z() {
        use tequila_core::math::Transform;
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        use tequila_scene::lighting::Photometry;
        use tequila_scene::render::RenderEngine;
        use tequila_scene::{Command, CommandBus};
        // A downlight about its aim, as bright as the panel: lit to 90
        // degrees off the aim, dark beyond.
        let ies = scratch("tequila-light-facing-file").join("downlight.ies");
        std::fs::write(&ies, concat!(
            "IESNA:LM-63-2002\n",
            "TILT=NONE\n",
            "1 -1 1 5 2 1 2 0 0 0\n",
            "1 1 0\n",
            "0 45 90 135 180\n",
            "0 360\n",
            "300 300 300 0 0\n",
            "300 300 300 0 0\n",
        )).unwrap();
        let plastic = tequila_scene::material::MaterialDefinition::new(
            "Plastic",
            tequila_materials::material_type::MaterialType::Plastic,
            Default::default(),
        );

        let mut engines = vec![RenderEngine::Tequila];
        match tequila_render_cycles::available() {
            Ok(()) => engines.push(RenderEngine::CyclesCpu),
            Err(why) => eprintln!("Cycles skipped: {why}"),
        }
        for file in [false, true] {
            let mut facing_down = board(&plastic, 1.0);
            let light = facing_down.tree.iter().find(|n| n.kind.light().is_some()).unwrap().id;
            if file {
                let mut settings = facing_down.tree.get(light).unwrap().kind.light().unwrap().clone();
                settings.photometry = Some(Photometry { path: ies.to_string_lossy().to_string() });
                CommandBus::new().apply(&mut facing_down, Command::SetLight { node: light, settings: Box::new(settings) }).unwrap();
            }
            let mut facing_up = facing_down.clone();
            let turned = facing_up.tree.get(light).unwrap().transform * Transform::rotation(Vec3::X, std::f32::consts::PI);
            CommandBus::new().apply(&mut facing_up, Command::set_transform(light, turned)).unwrap();
            let what = if file { "the panel with a file" } else { "the panel" };
            for &engine in &engines {
                let render = |name: &str, document: &tequila_scene::SceneDocument| -> f64 {
                    let black = Arc::new(Environment::uniform(Spectrum::ZERO));
                    let (mut scene, _) = tequila_scene::snapshot::snapshot(document, black, &mut tequila_scene::snapshot::MeshCache::new());
                    scene.background = Background::Color(Spectrum::ZERO);
                    let directory = scratch(&format!("tequila-light-facing-{}-{file}-{name}", engine.key()));
                    let renderer = BatchRenderer::new();
                    let mut job = job(1, &directory, RenderSettings { engine, width: 64, height: 48, max_samples: 32, denoise: false, ..Default::default() });
                    job.scene = Arc::new(scene);
                    job.camera = document.cameras.active().camera;
                    renderer.submit(job);
                    match wait_for_finish(&renderer) {
                        BatchEvent::Finished { preview, .. } => {
                            preview.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>()
                                / (preview.len() / 4 * 3) as f64
                        }
                        BatchEvent::Failed { message, .. } => panic!("{name}: {message}"),
                        _ => unreachable!(),
                    }
                };
                let (down, up) = (render("down", &facing_down), render("up", &facing_up));
                eprintln!("{}: {what}, +Z down averages {down:.1}, -Z down {up:.1}", engine.key());
                assert!(down > 20.0, "{}: {what} with +Z down left the board black ({down:.1})", engine.key());
                assert!(up < down * 0.1, "{}: {what}, turned away, still lit the board ({up:.1} against {down:.1})", engine.key());
            }
        }
    }

    /// A shadow-catcher floor catches the shadow a lamp casts in Cycles as it
    /// does in Tequila's renderer: letting the lamp cast shadows darkens the
    /// image by much the same amount in both. Cycles counts on a catcher only
    /// the lights marked as catcher lights, and while none were, a lamp's
    /// shadow went missing and an HDRI's sun cast half the shadow Blender's
    /// does. Skipped where Cycles is not installed.
    ///
    /// Twice: with a lamp that outshines the sky, and with one that lights
    /// the floor about as much as the sky does. Tequila's catcher used to
    /// average the quotients of one sky and one lamp sample at a time, which
    /// darkened the dimmer lamp's shadow by 15.5 where Cycles' darkened by
    /// 10.0, and the brighter one's by 19.1 against 17.6.
    #[test]
    fn a_cycles_shadow_catcher_catches_the_shadow() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        use tequila_render_core::light::{Light, LightKind, LightSet, PointLight};
        use tequila_render_core::scene::ShadowCatcher;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        for intensity in [20.0, 80.0] {
            // Off to the side and low, so the shadow falls on the floor beside
            // the sphere, in view.
            let mut lamp = Light::new(LightKind::Point(PointLight { position: vec3(-3.0, 3.0, 1.0), intensity: Spectrum::splat(intensity), radius: 0.05 }));
            let mut darkening = Vec::new();
            for engine in [tequila_scene::render::RenderEngine::Tequila, tequila_scene::render::RenderEngine::CyclesCpu] {
                let mut means = Vec::new();
                // The lamp's own shadow switched off and on: all that changes
                // is the shadow on the floor.
                for shadow in [false, true] {
                    lamp.visibility.shadow = shadow;
                    let directory = scratch(&format!("tequila-cycles-catcher-{intensity}-{}-{shadow}", engine.key()));
                    let renderer = BatchRenderer::new();
                    let mut render = job(1, &directory, RenderSettings {
                        engine,
                        width: 64, height: 48, max_samples: 256, denoise: false,
                        ..Default::default()
                    });
                    let mut scene = (*render.scene).clone().with_lights(Arc::new(LightSet::new(vec![lamp.clone()])));
                    scene.environment = Arc::new(Environment::uniform(Spectrum::splat(0.2)));
                    scene.background = Background::Environment;
                    scene.instances[1].shadow_catcher = Some(ShadowCatcher::default());
                    // A black sphere: nothing it reflects reaches the floor.
                    // Cycles counts light the sphere bounces into its own
                    // shadow, as a real floor would receive it; Tequila's
                    // catcher counts only what comes straight from the lights,
                    // so with a lit sphere the two shadows differ by the bounce
                    // and not by the lamp.
                    let m = &mut scene.materials[0];
                    m.base_color = Spectrum::ZERO;
                    m.specular_weight = 0.0;
                    m.coat_weight = 0.0;
                    scene.programs.clear();
                    render.scene = Arc::new(scene);
                    // The test job's quick settings leave shadows out of
                    // Tequila's renderer altogether.
                    render.config.shadows = true;
                    renderer.submit(render);
                    match wait_for_finish(&renderer) {
                        BatchEvent::Finished { preview, .. } => {
                            let mean = preview.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>()
                                / (preview.len() / 4 * 3) as f64;
                            means.push(mean);
                        }
                        BatchEvent::Failed { message, .. } => panic!("{message}"),
                        _ => unreachable!(),
                    }
                }
                darkening.push(means[0] - means[1]);
            }
            let (tequila, cycles) = (darkening[0], darkening[1]);
            assert!(tequila > 2.0, "the lamp cast no shadow in Tequila's render: it darkened it by {tequila:.2}");
            assert!(
                (cycles / tequila - 1.0).abs() < 0.04,
                "with a lamp of {intensity} cd, the shadow darkened Tequila's render by {tequila:.2} and Cycles' by {cycles:.2}"
            );
        }
    }

    /// A floor's bounce is seen by the product and never by the camera: a
    /// mirror sphere on a floor that catches no shadow, in a black studio
    /// with no lamp, mirrors black below its middle without one and the glow
    /// with one, while the pixels the camera sees of the floor itself stay
    /// as they were. In Tequila's renderer and, where it is installed, in
    /// Cycles.
    #[test]
    fn a_floor_bounce_lights_the_product_and_not_the_picture() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        use tequila_render_core::light::LightSet;
        use tequila_render_core::scene::ShadowCatcher;
        use tequila_scene::render::RenderEngine;
        let mut engines = vec![RenderEngine::Tequila];
        match tequila_render_cycles::available() {
            Ok(()) => engines.push(RenderEngine::CyclesCpu),
            Err(why) => eprintln!("Cycles skipped: {why}"),
        }
        for engine in engines {
            let picture = |bounce: Option<Spectrum>| -> Vec<u8> {
                let directory = scratch(&format!("tequila-bounce-{}-{}", engine.key(), bounce.is_some()));
                let renderer = BatchRenderer::new();
                let mut render =
                    job(1, &directory, RenderSettings { engine, width: 64, height: 48, max_samples: 64, denoise: false, ..Default::default() });
                let mut scene = (*render.scene).clone().with_lights(Arc::new(LightSet::new(Vec::new())));
                scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
                scene.background = Background::Environment;
                scene.instances[1].shadow_catcher = Some(ShadowCatcher { shadows: false, bounce, bounce_falloff: 0.0, ..ShadowCatcher::default() });
                let m = &mut scene.materials[0];
                m.base_color = Spectrum::splat(1.0);
                m.base_metalness = 1.0;
                m.specular_roughness = 0.05;
                m.coat_weight = 0.0;
                scene.programs.clear();
                render.scene = Arc::new(scene);
                renderer.submit(render);
                match wait_for_finish(&renderer) {
                    BatchEvent::Finished { preview, .. } => preview,
                    BatchEvent::Failed { message, .. } => panic!("{message}"),
                    _ => unreachable!(),
                }
            };
            let (dark, lifted) = (picture(None), picture(Some(Spectrum::splat(1.0))));
            let mean = |p: &[u8]| p.chunks_exact(4).map(|c| f64::from(c[0]) + f64::from(c[1]) + f64::from(c[2])).sum::<f64>() / (p.len() / 4) as f64;
            assert!(mean(&lifted) > mean(&dark) + 2.0, "{engine:?}: the bounce lit nothing, {:.2} against {:.2}", mean(&lifted), mean(&dark));
            // The floor's corners, where the sphere is not: the camera sees
            // no more of the floor than before.
            for corner in [0, 63, 64 * 47, 64 * 48 - 1] {
                let (a, b) = (&dark[4 * corner..4 * corner + 4], &lifted[4 * corner..4 * corner + 4]);
                assert!(a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= 2), "{engine:?}: the camera saw the bounce at pixel {corner}: {a:?} then {b:?}");
            }
        }
    }

    /// Rounded edges catch light a sharp edge cannot: a 2 cm white cube lit
    /// from in front and above, at 45 degrees, in a black studio. Its top and
    /// front faces see the lamp at 45 degrees; an edge rounded to 2 mm turns
    /// squarely to it and is the brightest thing in the picture, where the
    /// sharp edge has nothing brighter than the faces. In Tequila's renderer,
    /// and in Cycles where it is installed. (A lamp behind a face's plane
    /// lights it through no rounding: Cycles takes no light from below a
    /// surface's true face.)
    #[test]
    fn rounded_edges_catch_light_in_both_renderers() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        use tequila_render_core::light::{Light, LightKind, LightSet, PointLight};
        use tequila_render_core::scene::Mesh;
        use tequila_scene::render::RenderEngine;
        // A closed cube a centimetre either side of its centre, flat-shaded.
        let cube = {
            let (mut positions, mut normals, mut indices) = (Vec::new(), Vec::new(), Vec::new());
            for axis in 0..3 {
                for sign in [-1.0f32, 1.0] {
                    let n = [Vec3::X, Vec3::Y, Vec3::Z][axis] * sign;
                    let (u, v) = match axis {
                        0 => (Vec3::Y, Vec3::Z),
                        1 => (Vec3::Z, Vec3::X),
                        _ => (Vec3::X, Vec3::Y),
                    };
                    let (u, v) = if sign > 0.0 { (u, v) } else { (v, u) };
                    let base = positions.len() as u32;
                    for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                        positions.push((n + u * a + v * b) * 0.01);
                        normals.push(n);
                    }
                    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
                }
            }
            Mesh::new(positions, indices).with_normals(normals)
        };
        let lit = |engine: RenderEngine, radius: f32| -> f64 {
            let directory = scratch(&format!("tequila-bevel-{}-{radius}", engine.key()));
            let renderer = BatchRenderer::new();
            let mut render = job(
                1,
                &directory,
                RenderSettings { engine, width: 96, height: 72, max_samples: 64, denoise: false, ..Default::default() },
            );
            let above = Light::new(LightKind::Point(PointLight { position: vec3(0.0, 0.02 + 0.2, 0.01 + 0.2), intensity: Spectrum::splat(0.06), radius: 0.0 }));
            let mut scene = (*render.scene).clone().with_lights(Arc::new(LightSet::new(vec![above])));
            scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
            scene.background = Background::Environment;
            scene.meshes[0] = Arc::new(cube.clone());
            scene.instances[0].transform = tequila_core::math::Transform::translation(vec3(0.0, 0.01, 0.0));
            scene.instances.truncate(1);
            let m = &mut scene.materials[0];
            m.base_color = Spectrum::splat(0.8);
            m.specular_weight = 0.0;
            m.coat_weight = 0.0;
            m.edge_radius = radius;
            scene.programs.clear();
            render.scene = Arc::new(scene);
            render.camera = Camera::look_at(vec3(0.0, 0.035, 0.06), vec3(0.0, 0.008, 0.0), Vec3::Y);
            renderer.submit(render);
            match wait_for_finish(&renderer) {
                // The brightest twenty pixels, on average: the edge band
                // when there is one, the faces otherwise.
                BatchEvent::Finished { preview, .. } => {
                    let mut levels: Vec<u32> = preview.chunks_exact(4).map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])).collect();
                    levels.sort_unstable_by(|a, b| b.cmp(a));
                    levels[..20].iter().map(|&l| f64::from(l)).sum::<f64>() / 20.0
                }
                BatchEvent::Failed { message, .. } => panic!("{message}"),
                _ => unreachable!(),
            }
        };
        let mut engines = vec![RenderEngine::Tequila];
        match tequila_render_cycles::available() {
            Ok(_) => engines.push(RenderEngine::CyclesCpu),
            Err(why) => eprintln!("Cycles skipped: {why}"),
        }
        for engine in engines {
            let (sharp, rounded) = (lit(engine, 0.0), lit(engine, 0.002));
            assert!(sharp > 60.0, "{engine:?}: the faces should be lit: {sharp:.1}");
            assert!(sharp < 600.0, "{engine:?}: the faces should not clip: {sharp:.1}");
            assert!(rounded > sharp * 1.12, "{engine:?}: rounded edges brightest at {rounded:.1}, sharp ones' brightest {sharp:.1}");
        }
    }

    /// The demo sphere as a clear medium, `inside` it, in a black studio lit
    /// by `lamps`, through `engine`: the picture's mean, 8-bit, summed over
    /// its channels.
    fn medium(engine: tequila_scene::render::RenderEngine, name: &str, lamps: Vec<tequila_render_core::light::Light>, inside: impl Fn(&mut tequila_materials::openpbr::Material)) -> f64 {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        use tequila_render_core::light::LightSet;
        let directory = scratch(&format!("tequila-medium-{name}-{}", engine.key()));
        let renderer = BatchRenderer::new();
        let mut render = job(1, &directory, RenderSettings { engine, width: 64, height: 48, max_samples: 96, denoise: false, ..Default::default() });
        let mut scene = (*render.scene).clone().with_lights(Arc::new(LightSet::new(lamps)));
        scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
        scene.background = Background::Environment;
        scene.instances.truncate(1);
        let m = &mut scene.materials[0];
        m.base_color = Spectrum::splat(0.96);
        m.base_metalness = 0.0;
        m.coat_weight = 0.0;
        m.specular_roughness = 0.3;
        m.specular_ior = 1.49;
        m.transmission_weight = 1.0;
        inside(m);
        scene.programs.clear();
        render.scene = Arc::new(scene);
        render.config = IntegratorConfig { max_transmission_bounces: 12, ..IntegratorConfig::product() };
        renderer.submit(render);
        match wait_for_finish(&renderer) {
            BatchEvent::Finished { preview, .. } => {
                preview.chunks_exact(4).map(|c| f64::from(c[0]) + f64::from(c[1]) + f64::from(c[2])).sum::<f64>() / (preview.len() / 4) as f64
            }
            BatchEvent::Failed { message, .. } => panic!("{message}"),
            _ => unreachable!(),
        }
    }

    /// A medium that glows from within lights itself in a black studio with
    /// no lamp - a lit light guide - and as much in Cycles as in Tequila's
    /// renderer; without the glow it is dark.
    #[test]
    fn a_glowing_medium_glows_in_both_renderers() {
        use tequila_core::spectrum::Spectrum;
        use tequila_scene::render::RenderEngine;
        let mut engines = vec![RenderEngine::Tequila];
        match tequila_render_cycles::available() {
            Ok(()) => engines.push(RenderEngine::CyclesCpu),
            Err(why) => eprintln!("Cycles skipped: {why}"),
        }
        let mut glowing = Vec::new();
        for engine in engines {
            let absorbing = |m: &mut tequila_materials::openpbr::Material| {
                // Blender's Volume Absorption 0.9 at density 600: 60 a metre.
                m.transmission_color = Spectrum::splat(0.9);
                m.transmission_depth = -(0.9f32.ln()) / 60.0;
            };
            let dark = medium(engine, "dark", Vec::new(), absorbing);
            let lit = medium(engine, "glow", Vec::new(), |m| {
                absorbing(m);
                m.volume_emission_color = Spectrum::splat(1.0);
                m.volume_emission_strength = 20.0;
            });
            assert!(dark < 1.0, "{engine:?}: dark without its glow, {dark:.2}");
            assert!(lit > dark + 20.0, "{engine:?}: the glow lit {lit:.2}");
            glowing.push(lit);
        }
        if let [tequila, cycles] = glowing[..] {
            assert!((cycles / tequila - 1.0).abs() < 0.25, "Tequila's glow {tequila:.2}, Cycles' {cycles:.2}");
        }
    }

    /// A lamp lights a scattering medium in Cycles as in Tequila's renderer:
    /// a milky sphere under a lamp in a black studio. Cycles' lamps never
    /// lit volume scattering, and the sphere was dark there.
    #[test]
    fn a_lamp_lights_a_scattering_medium_in_cycles() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::light::{Light, LightKind, PointLight};
        use tequila_scene::render::RenderEngine;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let lamp = || vec![Light::new(LightKind::Point(PointLight { position: vec3(1.0, 2.0, 2.0), intensity: Spectrum::splat(40.0), radius: 0.1 }))];
        let milky = |m: &mut tequila_materials::openpbr::Material| {
            m.transmission_color = Spectrum::splat(0.95);
            m.transmission_depth = 0.2;
            m.transmission_scatter = Spectrum::splat(0.8);
        };
        let tequila = medium(RenderEngine::Tequila, "milk", lamp(), milky);
        let cycles = medium(RenderEngine::CyclesCpu, "milk", lamp(), milky);
        assert!(tequila > 5.0, "the lamp lit Tequila's milky sphere by only {tequila:.2}");
        assert!(cycles > 0.5 * tequila, "the lamp lit Cycles' milky sphere {cycles:.2} against Tequila's {tequila:.2}");
    }

    /// The mean of each channel of the demo scene with a black sphere on a
    /// catcher floor set up as `catcher`, under a grey sky and a lamp off to
    /// the side casting a shadow or not, through `engine`.
    fn caught(
        engine: tequila_scene::render::RenderEngine,
        catcher: tequila_render_core::scene::ShadowCatcher,
        lamp_shadow: bool,
        name: &str,
    ) -> [f64; 3] {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        use tequila_render_core::light::{Light, LightKind, LightSet, PointLight};
        let mut lamp = Light::new(LightKind::Point(PointLight { position: vec3(-3.0, 3.0, 1.0), intensity: Spectrum::splat(80.0), radius: 0.05 }));
        lamp.visibility.shadow = lamp_shadow;
        let directory = scratch(&format!("tequila-cycles-{name}-{}-{lamp_shadow}", engine.key()));
        let renderer = BatchRenderer::new();
        let mut render = job(1, &directory, RenderSettings { engine, width: 64, height: 48, max_samples: 256, denoise: false, ..Default::default() });
        let mut scene = (*render.scene).clone().with_lights(Arc::new(LightSet::new(vec![lamp])));
        scene.environment = Arc::new(Environment::uniform(Spectrum::splat(0.2)));
        scene.background = Background::Environment;
        scene.instances[1].shadow_catcher = Some(catcher);
        let m = &mut scene.materials[0];
        m.base_color = Spectrum::ZERO;
        m.specular_weight = 0.0;
        m.coat_weight = 0.0;
        scene.programs.clear();
        render.scene = Arc::new(scene);
        render.config.shadows = true;
        renderer.submit(render);
        match wait_for_finish(&renderer) {
            BatchEvent::Finished { preview, .. } => {
                let n = (preview.len() / 4) as f64;
                let mut sum = [0.0; 3];
                for p in preview.chunks_exact(4) {
                    for c in 0..3 {
                        sum[c] += f64::from(p[c]);
                    }
                }
                sum.map(|s| s / n)
            }
            BatchEvent::Failed { message, .. } => panic!("{message}"),
            _ => unreachable!(),
        }
    }

    /// A catcher's shadow comes out of Cycles in its colour, as it does out
    /// of Tequila's renderer: a red shadow colour makes the lamp's shadow
    /// redden the floor by much the same in both. Cycles' catcher shadow is
    /// only ever black. Skipped where Cycles is not installed.
    #[test]
    fn a_tinted_ground_shadow_is_its_colour_in_cycles() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::scene::ShadowCatcher;
        use tequila_scene::render::RenderEngine;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let red = ShadowCatcher { shadow_color: Spectrum { r: 0.6, g: 0.1, b: 0.0 }, ..ShadowCatcher::default() };
        // How much redder than blue the shadow makes the picture.
        let reddening = |engine| {
            let (lit, shaded) = (caught(engine, red, false, "tint"), caught(engine, red, true, "tint"));
            (shaded[0] - lit[0]) - (shaded[2] - lit[2])
        };
        let (tequila, cycles) = (reddening(RenderEngine::Tequila), reddening(RenderEngine::CyclesCpu));
        assert!(tequila > 1.0, "the red shadow did not redden Tequila's render: {tequila:.2}");
        assert!((cycles / tequila - 1.0).abs() < 0.35, "the red shadow reddened Tequila's render by {tequila:.2} and Cycles' by {cycles:.2}");
    }

    /// An occlusion-only catcher darkens the floor in Cycles as in Tequila's
    /// renderer: by what is near, as much in both, and not at all by the
    /// lamp's cast shadow. Cycles caught the cast shadow instead. Skipped
    /// where Cycles is not installed.
    #[test]
    fn an_occlusion_only_ground_shadow_is_occlusion_in_cycles() {
        use tequila_render_core::scene::ShadowCatcher;
        use tequila_scene::render::RenderEngine;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let near = ShadowCatcher { occlusion_only: true, occlusion_radius: 1.0, ..ShadowCatcher::default() };
        let none = ShadowCatcher { occlusion_radius: 1e-3, ..near };
        let brightness = |c: [f64; 3]| c[0] + c[1] + c[2];
        let mut darkening = Vec::new();
        for engine in [RenderEngine::Tequila, RenderEngine::CyclesCpu] {
            let open = brightness(caught(engine, none, true, "occlusion-none"));
            let occluded = brightness(caught(engine, near, true, "occlusion-near"));
            let unshadowed = brightness(caught(engine, near, false, "occlusion-near"));
            assert!(
                (occluded - unshadowed).abs() < 1.0,
                "{engine:?}: the lamp's shadow changed an occlusion-only floor, from {unshadowed:.2} to {occluded:.2}"
            );
            darkening.push(open - occluded);
        }
        let (tequila, cycles) = (darkening[0], darkening[1]);
        assert!(tequila > 2.0, "occlusion darkened Tequila's render by only {tequila:.2}");
        assert!((cycles / tequila - 1.0).abs() < 0.35, "occlusion darkened Tequila's render by {tequila:.2} and Cycles' by {cycles:.2}");
    }

    /// A floor kept for its reflection, with Ground's Catch shadows unticked,
    /// catches no shadow: the lamp's shadow on or off leaves it the same, in
    /// Tequila's renderer and in Cycles, where the same floor catching
    /// shadows darkens. It was a full catcher all the same, and the shadow
    /// switched off showed under the reflection, in the viewport and the
    /// render. Cycles where it is installed.
    #[test]
    fn a_floor_kept_for_its_reflection_catches_no_shadow() {
        use tequila_render_core::scene::ShadowCatcher;
        use tequila_scene::render::RenderEngine;
        let reflecting = ShadowCatcher { reflections: 0.3, ..ShadowCatcher::default() };
        let shadowless = ShadowCatcher { shadows: false, ..reflecting };
        let brightness = |c: [f64; 3]| c[0] + c[1] + c[2];
        let mut engines = vec![RenderEngine::Tequila];
        match tequila_render_cycles::available() {
            Ok(()) => engines.push(RenderEngine::CyclesCpu),
            Err(why) => eprintln!("Cycles skipped: {why}"),
        }
        for engine in engines {
            // How much the lamp's shadow darkens the picture.
            let shadow = |catcher, name| brightness(caught(engine, catcher, false, name)) - brightness(caught(engine, catcher, true, name));
            let catching = shadow(reflecting, "reflecting");
            let left = shadow(shadowless, "reflecting-shadowless");
            assert!(catching > 2.0, "{engine:?}: the lamp's shadow darkened a catching floor by only {catching:.2}");
            assert!(left.abs() < 1.0, "{engine:?}: a floor that catches no shadow darkened by {left:.2} under the lamp's shadow");
        }
    }

    /// The demo scene through Cycles, lit by a plain grey sky it also shows,
    /// with `change` made to it: the developed image, and where `point` lands
    /// in it. For comparing the ground's effects with and without it.
    fn cycles_view_of(name: &str, change: impl FnOnce(&mut tequila_render_core::scene::SceneSnapshot), point: Vec3) -> (Vec<u8>, usize, usize) {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        let (width, height) = (96, 72);
        let directory = scratch(&format!("tequila-cycles-floor-{name}"));
        let renderer = BatchRenderer::new();
        let mut render = job(1, &directory, RenderSettings {
            engine: tequila_scene::render::RenderEngine::CyclesCpu,
            width, height, max_samples: 64, denoise: false,
            ..Default::default()
        });
        let mut scene = (*render.scene).clone();
        scene.environment = Arc::new(Environment::uniform(Spectrum::splat(0.5)));
        scene.background = Background::Environment;
        change(&mut scene);
        render.scene = Arc::new(scene);
        let film = render.camera.project(point, width as f32 / height as f32).expect("the point is in view");
        let (x, y) = ((film.x * width as f32) as usize, (film.y * height as f32) as usize);
        renderer.submit(render);
        match wait_for_finish(&renderer) {
            BatchEvent::Finished { preview, .. } => (preview, x, y),
            BatchEvent::Failed { message, .. } => panic!("{message}"),
            _ => unreachable!(),
        }
    }

    /// Mean red of the 5x5 pixels around (x, y) of a 96-wide image.
    fn around(preview: &[u8], x: usize, y: usize) -> f64 {
        let mut sum = 0.0;
        for dy in 0..5 {
            for dx in 0..5 {
                sum += f64::from(preview[((y + dy - 2) * 96 + x + dx - 2) * 4]);
            }
        }
        sum / 25.0
    }

    /// Something see-through standing on a shadow-catching floor looks as
    /// it would with no floor at all, where the floor behind it is in no
    /// shadow. Cycles' own composite of the floor counted the see-through
    /// part twice, and a glass bulb went a fifth darker wherever the floor
    /// was behind it, with a line where the floor ended. Skipped where
    /// Cycles is not installed.
    #[test]
    fn a_see_through_part_on_the_floor_looks_as_with_no_floor() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        // Low on the sphere: the camera's ray through it carries on down to
        // the floor far behind, well clear of the sphere's own shadow.
        let point = vec3(0.0, 1.2, 0.0);
        // Black and without a highlight, so all it shows is what is behind
        // it: lit or not (the floor stops the sky's light from below), the
        // sphere itself adds nothing to compare.
        let see_through = |scene: &mut tequila_render_core::scene::SceneSnapshot| {
            let m = &mut scene.materials[0];
            m.geometry_opacity = 0.3;
            m.base_color = tequila_core::spectrum::Spectrum::ZERO;
            m.specular_weight = 0.0;
            m.coat_weight = 0.0;
            scene.programs.clear();
        };
        let (floor, x, y) = cycles_view_of("see-through", |scene| {
            see_through(scene);
            scene.instances[1].shadow_catcher = Some(tequila_render_core::scene::ShadowCatcher::default());
        }, point);
        let (open, ..) = cycles_view_of("see-through-open", |scene| {
            see_through(scene);
            scene.instances.truncate(1);
        }, point);
        let (floor, open) = (around(&floor, x, y), around(&open, x, y));
        assert!(open > 20.0, "the sphere came out black: {open:.1}");
        assert!(
            (floor / open - 1.0).abs() < 0.05,
            "through the sphere, the floor behind shows {floor:.1} and the open sky behind {open:.1}"
        );
    }

    /// The shadow-catching floor is no surface to a reflection: a mirror
    /// ball on it shows the environment below the horizon, as Tequila's
    /// renderer has it, not a white floor. Skipped where Cycles is not
    /// installed.
    #[test]
    fn a_mirror_on_the_floor_reflects_the_environment() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        // Low on the front of the sphere, where it reflects the ground.
        let point = vec3(0.45, 0.3, 0.55);
        let mirror = |scene: &mut tequila_render_core::scene::SceneSnapshot| {
            let m = &mut scene.materials[0];
            m.base_color = tequila_core::spectrum::Spectrum::ONE;
            m.base_weight = 1.0;
            m.base_metalness = 1.0;
            m.specular_roughness = 0.0;
            scene.programs.clear();
        };
        let (floor, x, y) = cycles_view_of("mirror", |scene| {
            mirror(scene);
            scene.instances[1].shadow_catcher = Some(tequila_render_core::scene::ShadowCatcher::default());
        }, point);
        let (open, ..) = cycles_view_of("mirror-open", |scene| {
            mirror(scene);
            scene.instances.truncate(1);
        }, point);
        let (floor, open) = (around(&floor, x, y), around(&open, x, y));
        assert!(open > 20.0, "the mirror came out black: {open:.1}");
        assert!(
            (floor / open - 1.0).abs() < 0.05,
            "the mirror shows {floor:.1} over the floor and {open:.1} with none"
        );
    }

    /// The demo sphere as a clear thin-walled shell: a bulb's glass.
    fn thin_glass(scene: &mut tequila_render_core::scene::SceneSnapshot) {
        let m = &mut scene.materials[0];
        m.transmission_weight = 1.0;
        m.transmission_color = tequila_core::spectrum::Spectrum::ONE;
        m.geometry_thin_walled = true;
        m.specular_roughness = 0.0;
        m.specular_ior = 1.5;
        scene.programs.clear();
    }

    /// Flakes in thin-walled glass come out of Cycles as out of Tequila's
    /// renderer: red flakes, half the surface, turn a clear shell as red in
    /// both. Cycles left them out of thin glass. Skipped where Cycles is not
    /// installed.
    #[test]
    fn flakes_in_thin_glass_show_in_cycles() {
        use tequila_core::spectrum::Spectrum;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let [tequila, cycles] = both_engines("thin-flakes", |scene| {
            thin_glass(scene);
            let m = &mut scene.materials[0];
            m.flake_weight = 0.5;
            m.flake_color = Spectrum { r: 0.9, g: 0.05, b: 0.05 };
            m.flake_size = 0.001;
            m.flake_spread = 0.3;
            m.flake_roughness = 0.3;
        });
        let (a, b) = (channel_means(&tequila), channel_means(&cycles));
        eprintln!("Tequila {a:.1?}, Cycles {b:.1?}");
        assert!(a[0] - a[2] > 15.0, "Tequila's flakes did not redden the glass: {a:.1?}");
        for c in 0..3 {
            assert!((b[c] / a[c] - 1.0).abs() < 0.12, "Tequila's flaked glass is {a:.1?}, Cycles' {b:.1?}");
        }
    }

    /// An environment on a dome of a size shows behind the product in Cycles
    /// as in Tequila's renderer: a sky striped round its horizon, on a dome
    /// flattened to a floor below it, lands on the same pixels around the
    /// sphere in both. Cycles drew every dome infinitely far away. Skipped
    /// where Cycles is not installed.
    #[test]
    fn a_finite_dome_shows_as_in_tequila() {
        use tequila_render_core::environment::{Dome, Environment};
        use tequila_texture::image::{ColorSpace, Image};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let (w, h) = (192, 96);
        let mut data = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let v = if (x * 24 / w + y * 12 / h) % 2 == 0 { 1.0 } else { 0.05 };
                data.extend_from_slice(&[v, v, v]);
            }
        }
        let mut sky = Environment::new(Arc::new(Image { width: w, height: h, channels: 3, color_space: ColorSpace::LinearSrgb, data }), 1.0);
        sky.set_dome(Dome { radius: Some(6.0), height: 1.0, flatten: true });
        let sky = Arc::new(sky);
        let [tequila, cycles] = both_engines("dome", move |scene| {
            scene.environment = sky.clone();
        });
        let brightness = |image: &[u8]| -> Vec<f64> { image.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).collect() };
        let (a, b) = (brightness(&tequila), brightness(&cycles));
        let (ma, mb) = (a.iter().sum::<f64>() / a.len() as f64, b.iter().sum::<f64>() / b.len() as f64);
        let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(&b) {
            cov += (x - ma) * (y - mb);
            va += (x - ma) * (x - ma);
            vb += (y - mb) * (y - mb);
        }
        let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
        eprintln!("correlation {correlation:.3}");
        assert!(correlation > 0.9, "the dome lands differently in the two engines: correlation {correlation:.2}");
    }

    /// A backplate shows behind the product in Cycles as in Tequila's
    /// renderer, with no shadow catcher in the scene: a plate striped across
    /// the frame lands on the same pixels around a plastic sphere, and
    /// through clear glass, thin or solid, which shows the plate rather than
    /// the environment it would refract. Cycles showed the environment
    /// instead, around the sphere and through it.
    /// Skipped where Cycles is not installed.
    #[test]
    fn a_backplate_shows_behind_the_product_in_cycles() {
        use tequila_render_core::environment::Background;
        use tequila_texture::image::{ColorSpace, Image};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let (w, h) = (96, 72);
        let mut data = Vec::with_capacity(w * h * 3);
        for _ in 0..h {
            for x in 0..w {
                let v = if (x / 12) % 2 == 0 { 0.9 } else { 0.05 };
                data.extend_from_slice(&[v, v, v]);
            }
        }
        let plate = Arc::new(Image { width: w, height: h, channels: 3, color_space: ColorSpace::LinearSrgb, data });
        let brightness = |image: &[u8]| -> Vec<f64> { image.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).collect() };
        let correlation = |a: &[f64], b: &[f64]| {
            let (ma, mb) = (a.iter().sum::<f64>() / a.len() as f64, b.iter().sum::<f64>() / b.len() as f64);
            let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
            for (x, y) in a.iter().zip(b) {
                cov += (x - ma) * (y - mb);
                va += (x - ma) * (x - ma);
                vb += (y - mb) * (y - mb);
            }
            cov / (va.sqrt() * vb.sqrt()).max(1e-9)
        };
        let mut wrong = Vec::new();
        for glass in ["none", "thin", "solid"] {
            let plate = plate.clone();
            let [tequila, cycles] = both_engines(&format!("backplate-{glass}"), move |scene| {
                if glass != "none" {
                    thin_glass(scene);
                    scene.materials[0].geometry_thin_walled = glass == "thin";
                }
                scene.background = Background::Backplate { image: plate.clone() };
            });
            let whole = correlation(&brightness(&tequila), &brightness(&cycles));
            let inside = correlation(&inside_the_sphere(&tequila), &inside_the_sphere(&cycles));
            eprintln!("glass {glass}: whole frame {whole:.3}, inside the sphere {inside:.3}");
            if whole < 0.9 || (glass != "none" && inside < 0.8) {
                wrong.push(format!("glass {glass}: correlation {whole:.2} over the frame, {inside:.2} inside the sphere"));
            }
        }
        assert!(wrong.is_empty(), "backplates that differ: {wrong:#?}");
    }

    /// Rough thin-walled glass blurs what is behind it in Cycles as in
    /// Tequila's renderer: a sky striped black and white, seen through a
    /// thin glass sphere, keeps its stripes in the same places and loses as
    /// much of their contrast in both, at each roughness. Cycles passed
    /// light through any thin wall unblurred, frosted or not. Skipped where
    /// Cycles is not installed.
    #[test]
    fn rough_thin_glass_blurs_as_in_tequila() {
        use tequila_render_core::environment::Environment;
        use tequila_texture::image::{ColorSpace, Image};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        // Twenty-four stripes round the horizon, fifteen degrees each.
        let (w, h) = (192, 96);
        let mut data = Vec::with_capacity(w * h * 3);
        for _ in 0..h {
            for x in 0..w {
                let v = if (x * 24 / w) % 2 == 0 { 1.0 } else { 0.02 };
                data.extend_from_slice(&[v, v, v]);
            }
        }
        let sky = Arc::new(Environment::new(Arc::new(Image { width: w, height: h, channels: 3, color_space: ColorSpace::LinearSrgb, data }), 1.0));
        let mut wrong = Vec::new();
        for roughness in [0.0f32, 0.2, 0.5] {
            let sky = sky.clone();
            let [tequila, cycles] = both_engines(&format!("frost-{roughness}"), move |scene| {
                thin_glass(scene);
                scene.materials[0].specular_roughness = roughness;
                scene.environment = sky.clone();
            });
            let (a, b) = (inside_the_sphere(&tequila), inside_the_sphere(&cycles));
            let ((ma, sa), (mb, sb)) = (mean_and_spread(&a), mean_and_spread(&b));
            let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
            for (x, y) in a.iter().zip(&b) {
                cov += (x - ma) * (y - mb);
                va += (x - ma) * (x - ma);
                vb += (y - mb) * (y - mb);
            }
            let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
            eprintln!("roughness {roughness}: correlation {correlation:.3}, mean {ma:.1} vs {mb:.1}, spread {sa:.1} vs {sb:.1}");
            // Where Tequila's renderer has blurred the stripes away there is
            // no pattern left to correlate, only noise.
            if (sa > 60.0 && correlation < 0.8) || (sb / sa - 1.0).abs() > 0.25 || (mb / ma - 1.0).abs() > 0.1 {
                wrong.push(format!("roughness {roughness}: correlation {correlation:.2}, Tequila {ma:.1} bright spreading {sa:.1}, Cycles {mb:.1} spreading {sb:.1}"));
            }
        }
        assert!(wrong.is_empty(), "thin glass that differs: {wrong:#?}");
    }

    /// Glass with an Abbe number splits a striped sky into colours in
    /// Cycles as in Tequila's renderer: through a solid glass sphere the
    /// stripes' edges take colour in both, where without one they stay grey,
    /// and the glass as a whole is no more tinted for it. Cycles dropped the
    /// Abbe number. Skipped where Cycles is not installed.
    #[test]
    fn dispersive_glass_splits_colour_in_cycles() {
        use tequila_render_core::environment::Environment;
        use tequila_texture::image::{ColorSpace, Image};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let (w, h) = (192, 96);
        let mut data = Vec::with_capacity(w * h * 3);
        for _ in 0..h {
            for x in 0..w {
                let v = if (x * 24 / w) % 2 == 0 { 1.0 } else { 0.02 };
                data.extend_from_slice(&[v, v, v]);
            }
        }
        let sky = Arc::new(Environment::new(Arc::new(Image { width: w, height: h, channels: 3, color_space: ColorSpace::LinearSrgb, data }), 1.0));
        let glass = |abbe: f32| {
            let sky = sky.clone();
            move |scene: &mut tequila_render_core::scene::SceneSnapshot| {
                let m = &mut scene.materials[0];
                m.transmission_weight = 1.0;
                m.transmission_color = tequila_core::spectrum::Spectrum::ONE;
                m.geometry_thin_walled = false;
                m.specular_roughness = 0.0;
                m.specular_ior = 1.6;
                m.transmission_dispersion_abbe = abbe;
                scene.programs.clear();
                scene.environment = sky.clone();
            }
        };
        // How far apart red and blue are, on average, inside the sphere.
        let colourful = |image: &[u8]| {
            let parted: Vec<u8> = image.chunks_exact(4).flat_map(|p| [p[0].abs_diff(p[2]), 0, 0, 255]).collect();
            let v = inside_the_sphere(&parted);
            v.iter().sum::<f64>() / v.len() as f64
        };
        let plain = both_engines("dispersion-none", glass(0.0));
        let split = both_engines("dispersion-abbe", glass(8.0));
        for (engine, name) in [(0, "Tequila's renderer"), (1, "Cycles")] {
            let (before, after) = (colourful(&plain[engine]), colourful(&split[engine]));
            let (a, b) = (channel_means(&plain[engine]), channel_means(&split[engine]));
            eprintln!("{name}: red from blue {before:.1} to {after:.1}; means {a:.1?} to {b:.1?}");
            assert!(after > before + 3.0 && after > 1.5 * before, "{name}: the glass split no colour ({before:.1} to {after:.1})");
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() < 0.15 * a[c].max(20.0), "{name}: dispersion tinted the glass, {a:.1?} to {b:.1?}");
            }
        }
    }

    /// A thin-walled shell passes light straight through, in Cycles as in
    /// Tequila's renderer, so a clear one casts a faint shadow where a
    /// solid sphere casts a full one. Cycles' principled glass is always a
    /// solid's, bending light and stopping shadows. Skipped where Cycles is
    /// not installed.
    #[test]
    fn a_thin_glass_shell_casts_a_faint_shadow() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        use tequila_render_core::light::{Light, LightKind, LightSet, PointLight};
        use tequila_render_core::scene::ShadowCatcher;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let mut lamp = Light::new(LightKind::Point(PointLight { position: vec3(-3.0, 3.0, 1.0), intensity: Spectrum::splat(20.0), radius: 0.05 }));
        let mut darkening = Vec::new();
        for glass in [false, true] {
            let mut means = Vec::new();
            for shadow in [false, true] {
                lamp.visibility.shadow = shadow;
                let directory = scratch(&format!("tequila-cycles-thin-shadow-{glass}-{shadow}"));
                let renderer = BatchRenderer::new();
                let mut render = job(1, &directory, RenderSettings {
                    engine: tequila_scene::render::RenderEngine::CyclesCpu,
                    width: 64, height: 48, max_samples: 128, denoise: false,
                    ..Default::default()
                });
                let mut scene = (*render.scene).clone().with_lights(Arc::new(LightSet::new(vec![lamp.clone()])));
                scene.environment = Arc::new(Environment::uniform(Spectrum::splat(0.2)));
                scene.background = Background::Environment;
                scene.instances[1].shadow_catcher = Some(ShadowCatcher::default());
                if glass {
                    thin_glass(&mut scene);
                }
                render.scene = Arc::new(scene);
                renderer.submit(render);
                match wait_for_finish(&renderer) {
                    BatchEvent::Finished { preview, .. } => {
                        let mean = preview.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>()
                            / (preview.len() / 4 * 3) as f64;
                        means.push(mean);
                    }
                    BatchEvent::Failed { message, .. } => panic!("{message}"),
                    _ => unreachable!(),
                }
            }
            darkening.push(means[0] - means[1]);
        }
        let (solid, shell) = (darkening[0], darkening[1]);
        assert!(solid > 2.0, "the solid sphere cast no shadow: it darkened the image by {solid:.2}");
        assert!(
            shell < 0.35 * solid,
            "the thin shell's shadow darkened the image by {shell:.2}, the solid sphere's by {solid:.2}"
        );
    }

    /// Seen through near its rim, a clear thin shell shows what is behind
    /// it. Cycles' Fresnel inverts the index on a surface's far side, and
    /// light inside the shell met a mirror there: the rim of a bulb came out
    /// black. Skipped where Cycles is not installed.
    #[test]
    fn a_thin_glass_shell_is_clear_to_its_rim() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        // Nine tenths of the way out from the sphere's centre, sideways to
        // the camera: through the shell at a grazing angle.
        let eye = vec3(2.0, 1.5, 3.0);
        let centre = vec3(0.0, 1.0, 0.0);
        let across = (centre - eye).cross(Vec3::Y).normalize();
        let point = centre + across * 0.9;
        let (shell, x, y) = cycles_view_of("thin-rim", |scene| {
            thin_glass(scene);
            scene.instances.truncate(1);
        }, point);
        let (open, ..) = cycles_view_of("thin-rim-open", |scene| {
            scene.instances.clear();
        }, point);
        let (shell, open) = (around(&shell, x, y), around(&open, x, y));
        assert!(open > 20.0, "the sky came out black: {open:.1}");
        assert!(
            (shell / open - 1.0).abs() < 0.15,
            "near its rim the shell shows {shell:.1}, the sky behind it {open:.1}"
        );
    }

    /// A reflecting floor shows the part standing on it, in Cycles as in
    /// Tequila's renderer: seen low across the floor, where it reflects
    /// most, the floor below the red sphere turns red. Cycles' catcher
    /// reflected nothing. Skipped where Cycles is not installed.
    #[test]
    fn a_reflecting_floor_shows_the_part_on_it() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        use tequila_render_core::scene::ShadowCatcher;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let (width, height) = (96, 72);
        let camera = Camera::look_at(vec3(4.0, 0.4, 4.0), vec3(0.0, 0.6, 0.0), Vec3::Y);
        // Where the floor mirrors the sphere's lower front: the camera's
        // ray to that point's reflection below the floor crosses it here.
        let mirrored = vec3(0.0, -0.3, 0.7);
        let eye = vec3(4.0, 0.4, 4.0);
        let t = eye.y / (eye.y - mirrored.y);
        let point = eye + (mirrored - eye) * t;
        let film = camera.project(point, width as f32 / height as f32).expect("the floor point is in view");
        let (x, y) = ((film.x * width as f32) as usize, (film.y * height as f32) as usize);
        // Red over green around (x, y): the sphere's colour in the floor.
        let redness = |preview: &[u8]| -> f64 {
            let (mut r, mut g) = (0.0, 0.0);
            for dy in 0..5 {
                for dx in 0..5 {
                    let i = ((y + dy - 2) * width + x + dx - 2) * 4;
                    r += f64::from(preview[i]);
                    g += f64::from(preview[i + 1]);
                }
            }
            r / g.max(1.0)
        };
        let mut shift = Vec::new();
        for engine in [tequila_scene::render::RenderEngine::Tequila, tequila_scene::render::RenderEngine::CyclesCpu] {
            let mut seen = Vec::new();
            for reflections in [0.0, 1.0] {
                let directory = scratch(&format!("tequila-cycles-reflect-{}-{reflections}", engine.key()));
                let renderer = BatchRenderer::new();
                let mut render = job(1, &directory, RenderSettings {
                    engine,
                    width, height, max_samples: 128, denoise: false,
                    ..Default::default()
                });
                render.camera = camera;
                let mut scene = (*render.scene).clone();
                scene.environment = Arc::new(Environment::uniform(Spectrum::splat(0.5)));
                scene.background = Background::Environment;
                scene.instances[1].shadow_catcher = Some(ShadowCatcher { reflections, ..ShadowCatcher::default() });
                render.scene = Arc::new(scene);
                render.config.shadows = true;
                renderer.submit(render);
                match wait_for_finish(&renderer) {
                    BatchEvent::Finished { preview, .. } => seen.push(redness(&preview)),
                    BatchEvent::Failed { message, .. } => panic!("{message}"),
                    _ => unreachable!(),
                }
            }
            shift.push(seen[1] / seen[0]);
        }
        let (tequila, cycles) = (shift[0], shift[1]);
        assert!(tequila > 1.05, "Tequila's floor did not turn red: red over green rose {tequila:.3} times");
        assert!(cycles > 1.1, "Cycles' floor did not turn red: red over green rose {cycles:.3} times, Tequila's {tequila:.3}");
    }

    /// Gold comes out of Cycles the colour it comes out of Tequila's
    /// renderer. Its reflectance is a measured index (n, k), tinted, and is
    /// more saturated than Rec.709 can hold: while Cycles worked in Rec.709
    /// and took a metal's colour as its base colour, it clamped gold's red
    /// and the index went unused, and gold rendered pale yellow. Skipped
    /// where Cycles is not installed.
    #[test]
    fn gold_is_the_same_colour_in_cycles() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let (width, height) = (64, 48);
        let mut colour = Vec::new();
        for engine in [tequila_scene::render::RenderEngine::Tequila, tequila_scene::render::RenderEngine::CyclesCpu] {
            let directory = scratch(&format!("tequila-cycles-gold-{}", engine.key()));
            let renderer = BatchRenderer::new();
            let mut render = job(1, &directory, RenderSettings {
                engine,
                width, height, max_samples: 64, denoise: false,
                ..Default::default()
            });
            let mut scene = (*render.scene).clone();
            scene.environment = Arc::new(Environment::uniform(Spectrum::splat(0.5)));
            scene.background = Background::Color(Spectrum::ZERO);
            let mut gold = tequila_materials::openpbr::Material::metal(tequila_materials::fresnel::metals::GOLD, 0.2);
            gold.base_color = Spectrum { r: 1.0, g: 0.78, b: 0.34 };
            scene.materials[0] = gold;
            scene.programs.clear();
            scene.instances.truncate(1);
            render.scene = Arc::new(scene);
            renderer.submit(render);
            let preview = match wait_for_finish(&renderer) {
                BatchEvent::Finished { preview, .. } => preview,
                BatchEvent::Failed { message, .. } => panic!("{message}"),
                _ => unreachable!(),
            };
            // The sphere's middle, where it reflects the sky head on.
            let (mut sum, mut n) = ([0.0f64; 3], 0.0);
            for y in height / 2 - 4..height / 2 + 4 {
                for x in width / 2 - 4..width / 2 + 4 {
                    let i = (y * width + x) * 4;
                    for c in 0..3 {
                        sum[c] += f64::from(preview[i + c]);
                    }
                    n += 1.0;
                }
            }
            colour.push(sum.map(|s| s / n));
        }
        let (tequila, cycles) = (colour[0], colour[1]);
        assert!(tequila[0] > 40.0, "the gold sphere came out black: {tequila:?}");
        for c in 0..3 {
            assert!((tequila[c] - cycles[c]).abs() < 8.0, "Tequila's gold is {tequila:.1?}, Cycles' {cycles:.1?}");
        }
    }

    /// The demo scene through both engines, with `change` made to it, under
    /// a plain grey sky it shows: the two developed images.
    fn both_engines(name: &str, change: impl Fn(&mut tequila_render_core::scene::SceneSnapshot)) -> [Vec<u8>; 2] {
        both_engines_scaled(name, 1.0, change)
    }

    /// As [`both_engines`], the whole scene and the camera with it `scale`
    /// times the size: the same picture of a smaller or larger world.
    fn both_engines_scaled(name: &str, scale: f32, change: impl Fn(&mut tequila_render_core::scene::SceneSnapshot)) -> [Vec<u8>; 2] {
        both_engines_lit(name, scale, Lit { config: IntegratorConfig::product(), reject_fireflies: true, camera: None }, change)
    }

    /// The lighting [`both_engines_lit`] renders under, and the camera it
    /// looks through when not the test job's.
    struct Lit {
        config: IntegratorConfig,
        reject_fireflies: bool,
        camera: Option<tequila_render_core::camera::Camera>,
    }

    /// As [`both_engines_scaled`], under `lit`'s lighting.
    fn both_engines_lit(name: &str, scale: f32, lit: Lit, change: impl Fn(&mut tequila_render_core::scene::SceneSnapshot)) -> [Vec<u8>; 2] {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::{Background, Environment};
        let mut images = Vec::new();
        for engine in [tequila_scene::render::RenderEngine::Tequila, tequila_scene::render::RenderEngine::CyclesCpu] {
            let directory = scratch(&format!("tequila-cycles-{name}-{}", engine.key()));
            let renderer = BatchRenderer::new();
            let mut render = job(1, &directory, RenderSettings {
                engine,
                width: 96, height: 72, max_samples: 64, denoise: false,
                reject_fireflies: lit.reject_fireflies,
                ..Default::default()
            });
            let mut scene = (*render.scene).clone();
            scene.environment = Arc::new(Environment::uniform(Spectrum::splat(0.5)));
            scene.background = Background::Environment;
            scene.instances.truncate(1);
            change(&mut scene);
            if scale != 1.0 {
                let resize = tequila_core::math::Transform::from_matrix(tequila_core::math::Mat4::scale(vec3(scale, scale, scale))).expect("invertible");
                for instance in &mut scene.instances {
                    instance.transform = resize * instance.transform;
                }
                render.camera = tequila_render_core::camera::Camera::look_at(vec3(2.0, 1.5, 3.0) * scale, vec3(0.0, 0.5, 0.0) * scale, Vec3::Y);
            }
            if let Some(camera) = lit.camera {
                render.camera = camera;
            }
            render.scene = Arc::new(scene);
            // The everyday preset, not the test job's quick one: light
            // passes through glass.
            render.config = lit.config.clone();
            renderer.submit(render);
            match wait_for_finish(&renderer) {
                BatchEvent::Finished { preview, .. } => images.push(preview),
                BatchEvent::Failed { message, .. } => panic!("{message}"),
                _ => unreachable!(),
            }
        }
        [images.remove(0), images.remove(0)]
    }

    /// A label lands on a part in Cycles where it lands in Tequila's
    /// renderer: a red stripe wrapped round the sphere by a cylindrical
    /// projection - scaled, offset, clear outside its artwork - covers the
    /// same pixels in both. Cycles dropped labels altogether, and read every
    /// projection as the mesh's UVs. Skipped where Cycles is not installed.
    #[test]
    fn a_label_lands_where_tequila_puts_it() {
        use tequila_core::spectrum::Spectrum;
        use tequila_materials::graph::{MaterialGraph, TextureId};
        use tequila_materials::material_type::MaterialType;
        use tequila_texture::image::{ColorSpace, Image};
        use tequila_texture::projection::{Axis, Mapping, Projection};
        use tequila_texture::texture::{MapRole, Texture};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let labelled = |scene: &mut tequila_render_core::scene::SceneSnapshot| {
            let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
            graph.material_mut().base_color = Spectrum::splat(0.8);
            // Red on the left half of the artwork, nothing on the right - but
            // white in its colour, as artwork saved straight often is there.
            let mut stripe = tequila_materials::decal::Decal::new("Stripe", TextureId(1));
            stripe.mapping = Mapping {
                projection: Projection::Cylindrical,
                axis: Axis::Y,
                // The front of the sphere, from the camera, is near u = 0.66:
                // red from 0.6 to 0.75, clear from there to 0.9, nothing else.
                scale: tequila_core::math::vec3(1.0 / 0.3, 0.25, 1.0),
                offset: tequila_core::math::vec2(-2.0, 0.5),
                ..Mapping::default()
            };
            graph.decals.push(stripe);
            let program = graph.compile().expect("compiles");
            let mut table = tequila_render_core::shade::TextureTable::new();
            table.insert(TextureId(1), Arc::new(Texture::from_linear(
                Image { width: 2, height: 1, channels: 4, color_space: ColorSpace::LinearSrgb, data: vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0] },
                MapRole::Color,
                Mapping::default(),
            )));
            scene.materials[0] = graph.material().clone();
            *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                .with_programs(vec![Some(Arc::new(program))], Arc::new(table));
        };
        let [tequila, cycles] = both_engines("label", labelled);
        // How much redder than green each pixel is, and where.
        let redness = |image: &[u8]| -> Vec<f64> { image.chunks_exact(4).map(|p| f64::from(p[0]) - f64::from(p[1])).collect() };
        let (a, b) = (redness(&tequila), redness(&cycles));
        let reddest = a.iter().copied().fold(0.0, f64::max);
        assert!(reddest > 60.0, "Tequila's render shows no label: reddest {reddest:.0}");
        let covered = a.iter().filter(|&&r| r > reddest * 0.5).count();
        let agree = a.iter().zip(&b).filter(|(&x, &y)| x > reddest * 0.5 && y > reddest * 0.5).count();
        assert!(
            agree as f64 > 0.8 * covered as f64,
            "Cycles shows the label on {agree} of the {covered} pixels Tequila's render shows it on"
        );
        let stray = b.iter().zip(&a).filter(|(&y, &x)| y > reddest * 0.5 && x < reddest * 0.25).count();
        assert!(stray * 10 < covered, "Cycles shows the label on {stray} pixels where Tequila's render has none");
        // And no paler: the artwork's clear texels are white, which a label
        // filtered without its coverage folded in lets into the ink.
        let green = |image: &[u8]| -> f64 {
            let (sum, n) = image.chunks_exact(4).zip(&a).filter(|(_, &r)| r > reddest * 0.25).fold((0.0, 0.0), |(s, n), (p, _)| (s + f64::from(p[1]), n + 1.0));
            sum / n
        };
        let (g_tequila, g_cycles) = (green(&tequila), green(&cycles));
        assert!((g_tequila - g_cycles).abs() < 12.0, "where the label is, green is {g_tequila:.1} in Tequila's render and {g_cycles:.1} in Cycles'");
    }

    /// Colours painted on the vertices - a ZBrush polypaint, a scan's -
    /// show in Cycles as in Tequila's renderer: a sphere painted red above
    /// its middle and blue below, through a Vertex Color node, comes out red
    /// over blue in both, the same reds and blues. Cycles had no vertex
    /// colours and rendered such a part flat grey. Skipped where Cycles is
    /// not installed.
    #[test]
    fn vertex_colours_show_in_cycles_as_in_tequila() {
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket};
        use tequila_materials::material_type::{MaterialType, Param};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let painted = |scene: &mut tequila_render_core::scene::SceneSnapshot| {
            let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
            let output = graph.output();
            let colours = graph.add_node(Node::new(NodeKind::VertexColor));
            graph.connect(colours, output, Socket::Param(Param::BaseColor));
            let program = graph.compile().expect("compiles");
            scene.materials[0] = graph.material().clone();
            let table = tequila_render_core::shade::TextureTable::new();
            *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                .with_programs(vec![Some(Arc::new(program))], Arc::new(table));
            let index = scene.instances[0].mesh.index();
            let mut mesh = (*scene.meshes[index]).clone();
            let (low, high) = mesh.positions.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), p| (l.min(p.y), h.max(p.y)));
            let middle = (low + high) * 0.5;
            mesh.colors = mesh
                .positions
                .iter()
                .map(|p| if p.y > middle { tequila_core::math::vec4(0.8, 0.05, 0.05, 1.0) } else { tequila_core::math::vec4(0.05, 0.1, 0.8, 1.0) })
                .collect();
            scene.meshes[index] = Arc::new(mesh);
        };
        let [tequila, cycles] = both_engines("vertex-colours", painted);
        // Where each image is clearly red, and clearly blue.
        let classify = |image: &[u8]| -> (Vec<usize>, Vec<usize>) {
            let mut reds = Vec::new();
            let mut blues = Vec::new();
            for (i, p) in image.chunks_exact(4).enumerate() {
                let (r, g, b) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
                if r > g + 40 && r > b + 40 {
                    reds.push(i);
                } else if b > r + 40 && b > g + 20 {
                    blues.push(i);
                }
            }
            (reds, blues)
        };
        let (reds, blues) = classify(&tequila);
        assert!(reds.len() > 200 && blues.len() > 200, "Tequila's render is not red over blue: {} red, {} blue", reds.len(), blues.len());
        let (cycles_reds, cycles_blues): (Vec<usize>, Vec<usize>) = {
            let red = |i: &usize| {
                let p = &cycles[i * 4..i * 4 + 3];
                i32::from(p[0]) > i32::from(p[1]) + 40 && i32::from(p[0]) > i32::from(p[2]) + 40
            };
            let blue = |i: &usize| {
                let p = &cycles[i * 4..i * 4 + 3];
                i32::from(p[2]) > i32::from(p[0]) + 40 && i32::from(p[2]) > i32::from(p[1]) + 20
            };
            (reds.iter().copied().filter(red).collect(), blues.iter().copied().filter(blue).collect())
        };
        assert!(
            cycles_reds.len() * 10 > reds.len() * 8 && cycles_blues.len() * 10 > blues.len() * 8,
            "Cycles is red on {} of Tequila's {} red pixels and blue on {} of its {} blue ones",
            cycles_reds.len(), reds.len(), cycles_blues.len(), blues.len()
        );
        let mean = |image: &[u8], at: &[usize]| -> [f64; 3] {
            let mut sum = [0.0; 3];
            for &i in at {
                for c in 0..3 {
                    sum[c] += f64::from(image[i * 4 + c]);
                }
            }
            sum.map(|s| s / at.len().max(1) as f64)
        };
        for (name, at) in [("red", &reds), ("blue", &blues)] {
            let (a, b) = (mean(&tequila, at), mean(&cycles, at));
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() < 14.0, "the {name} is {a:.1?} in Tequila's render and {b:.1?} in Cycles'");
            }
        }
    }

    /// A label only partly covering fades in Cycles as it does in Tequila's
    /// renderer: cream ink at a quarter coverage and ink brighter than white
    /// at a half, all over a dark sphere, come out the same colour in both,
    /// one through each way a label goes to Cycles. Cycles squared the
    /// coverage of a white label, so a quarter-covered print all but
    /// vanished, and drew a cream one brighter than solid ink, so the fading
    /// rings of a pedal's silkscreen rendered solid. Skipped where Cycles is
    /// not installed.
    #[test]
    fn a_part_covering_label_fades_as_in_tequila() {
        use tequila_core::spectrum::Spectrum;
        use tequila_materials::graph::{MaterialGraph, TextureId};
        use tequila_materials::material_type::MaterialType;
        use tequila_texture::image::{ColorSpace, Image};
        use tequila_texture::projection::{Axis, Mapping, Projection};
        use tequila_texture::texture::{MapRole, Texture};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        // The sphere's middle, where the camera looks.
        let middle = |image: &[u8]| -> [f64; 3] {
            let (mut sum, mut n) = ([0.0; 3], 0.0);
            for y in 30..38 {
                for x in 44..52 {
                    let i = (y * 96 + x) * 4;
                    for c in 0..3 {
                        sum[c] += f64::from(image[i + c]);
                    }
                    n += 1.0;
                }
            }
            sum.map(|s| s / n)
        };
        let dark = |scene: &mut tequila_render_core::scene::SceneSnapshot| {
            scene.materials[0].base_color = Spectrum::splat(0.05);
            scene.programs.clear();
        };
        let [bare, _] = both_engines("label-bare", dark);
        let bare = middle(&bare);
        // Cream goes to Cycles as bytes; ink brighter than white, as floats.
        for (name, ink, coverage) in [("cream", [0.8, 0.7, 0.5], 0.25), ("bright", [1.5; 3], 0.5)] {
            let labelled = |scene: &mut tequila_render_core::scene::SceneSnapshot| {
                let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
                graph.material_mut().base_color = Spectrum::splat(0.05);
                // All the way round the sphere and over its height.
                let mut print = tequila_materials::decal::Decal::new("Print", TextureId(1));
                print.mapping = Mapping {
                    projection: Projection::Cylindrical,
                    axis: Axis::Y,
                    scale: tequila_core::math::vec3(1.0, 0.25, 1.0),
                    offset: tequila_core::math::vec2(0.0, 0.5),
                    ..Mapping::default()
                };
                graph.decals.push(print);
                let program = graph.compile().expect("compiles");
                let mut table = tequila_render_core::shade::TextureTable::new();
                let texel = [ink[0], ink[1], ink[2], coverage];
                table.insert(TextureId(1), Arc::new(Texture::from_linear(
                    Image { width: 16, height: 16, channels: 4, color_space: ColorSpace::LinearSrgb, data: texel.repeat(256) },
                    MapRole::Color,
                    Mapping::default(),
                )));
                scene.materials[0] = graph.material().clone();
                *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                    .with_programs(vec![Some(Arc::new(program))], Arc::new(table));
            };
            let [tequila, cycles] = both_engines(&format!("label-{name}"), labelled);
            let (tequila, cycles) = (middle(&tequila), middle(&cycles));
            assert!(tequila[1] > bare[1] + 20.0, "Tequila's render shows no {name} label: {tequila:.1?} against {bare:.1?} bare");
            for c in 0..3 {
                assert!(
                    (tequila[c] - cycles[c]).abs() < 8.0,
                    "{name} ink at {coverage} coverage is {tequila:.1?} in Tequila's render and {cycles:.1?} in Cycles' ({bare:.1?} bare)"
                );
            }
        }
    }

    /// White artwork prints in its ink's colour, the same in both renderers:
    /// a label all round a dark sphere, white with a red ink, comes out red,
    /// and as red in Cycles as in Tequila's renderer. Skipped where Cycles is
    /// not installed.
    #[test]
    fn a_labels_ink_colours_white_artwork() {
        use tequila_core::spectrum::Spectrum;
        use tequila_materials::graph::{MaterialGraph, TextureId};
        use tequila_materials::material_type::MaterialType;
        use tequila_texture::image::{ColorSpace, Image};
        use tequila_texture::projection::{Axis, Mapping, Projection};
        use tequila_texture::texture::{MapRole, Texture};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let middle = |image: &[u8]| -> [f64; 3] {
            let (mut sum, mut n) = ([0.0; 3], 0.0);
            for y in 30..38 {
                for x in 44..52 {
                    for c in 0..3 {
                        sum[c] += f64::from(image[(y * 96 + x) * 4 + c]);
                    }
                    n += 1.0;
                }
            }
            sum.map(|s| s / n)
        };
        let inked = |scene: &mut tequila_render_core::scene::SceneSnapshot| {
            let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
            graph.material_mut().base_color = Spectrum::splat(0.05);
            let mut print = tequila_materials::decal::Decal::new("Print", TextureId(1));
            print.mapping = Mapping {
                projection: Projection::Cylindrical,
                axis: Axis::Y,
                scale: tequila_core::math::vec3(1.0, 0.25, 1.0),
                offset: tequila_core::math::vec2(0.0, 0.5),
                ..Mapping::default()
            };
            print.ink = Spectrum { r: 0.7, g: 0.08, b: 0.05 };
            graph.decals.push(print);
            let program = graph.compile().expect("compiles");
            let mut table = tequila_render_core::shade::TextureTable::new();
            table.insert(TextureId(1), Arc::new(Texture::from_linear(
                Image { width: 4, height: 4, channels: 4, color_space: ColorSpace::LinearSrgb, data: [1.0, 1.0, 1.0, 1.0].repeat(16) },
                MapRole::Color,
                Mapping::default(),
            )));
            scene.materials[0] = graph.material().clone();
            *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                .with_programs(vec![Some(Arc::new(program))], Arc::new(table));
        };
        let [tequila, cycles] = both_engines("label-ink", inked);
        let (tequila, cycles) = (middle(&tequila), middle(&cycles));
        assert!(tequila[0] > tequila[1] + 40.0, "white artwork in red ink is not red: {tequila:.1?}");
        for c in 0..3 {
            assert!((tequila[c] - cycles[c]).abs() < 10.0, "red ink is {tequila:.1?} in Tequila's render and {cycles:.1?} in Cycles'");
        }
    }

    /// A held-out part is a hole in the picture where the camera sees it,
    /// in both renderers: the sphere's middle goes from lit to nothing.
    /// Skipped where Cycles is not installed.
    #[test]
    fn a_held_out_part_is_a_hole_in_both_renderers() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let middle = |image: &[u8]| -> f64 {
            let (mut sum, mut n) = (0.0, 0.0);
            for y in 30..38 {
                for x in 44..52 {
                    let i = (y * 96 + x) * 4;
                    sum += f64::from(image[i]) + f64::from(image[i + 1]) + f64::from(image[i + 2]);
                    n += 1.0;
                }
            }
            sum / n
        };
        let [plain_t, plain_c] = both_engines("holdout-off", |_| {});
        let [held_t, held_c] = both_engines("holdout-on", |scene| scene.instances[0].holdout = true);
        for (name, plain, held) in [("Tequila's renderer", middle(&plain_t), middle(&held_t)), ("Cycles", middle(&plain_c), middle(&held_c))] {
            assert!(plain > 60.0, "{name}: the sphere is not lit: {plain:.1}");
            assert!(held < 8.0, "{name}: a held-out sphere still shows: {held:.1} where it was {plain:.1}");
        }
    }

    /// A light kept to some parts lights those and no others, in both
    /// renderers alike: with nothing else lighting the scene, a lamp kept to
    /// one of two spheres and the same lamp kept to the other add up to the
    /// lamp lighting both, each less than that, and Cycles agrees. Skipped
    /// where Cycles is not installed or predates light linking.
    #[test]
    fn a_linked_light_lights_only_its_parts() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::Environment;
        use tequila_render_core::light::{Light, LightKind, LightLinks, LightSet, PointLight};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let lit = |links: LightLinks| {
            move |scene: &mut tequila_render_core::scene::SceneSnapshot| {
                // Side by side across the view: the camera's right is
                // (3, 0, -2) normalized.
                let right = vec3(3.0, 0.0, -2.0).normalize() * 0.65;
                let mut second = scene.instances[0].clone();
                second.id = tequila_core::ids::InstanceId(1);
                second.transform = tequila_core::math::Transform::translation(right) * second.transform;
                scene.instances[0].transform = tequila_core::math::Transform::translation(-right) * scene.instances[0].transform;
                scene.instances.push(second);
                scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
                let mut lamp = Light::new(LightKind::Point(PointLight { position: vec3(0.5, 3.0, 2.5), intensity: Spectrum::splat(6.0), radius: 0.05 }));
                lamp.links = links.clone();
                *scene = scene.clone().with_lights(Arc::new(LightSet::new(vec![lamp])));
            }
        };
        let total = |image: &[u8]| image.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>();
        let shots = [
            both_engines("linked-all", lit(LightLinks::All)),
            both_engines("linked-first", lit(LightLinks::Only(Arc::from([0u32])))),
            both_engines("linked-second", lit(LightLinks::AllBut(Arc::from([0u32])))),
        ];
        for (engine, name) in [(0, "Tequila's renderer"), (1, "Cycles")] {
            let [all, first, second] = [&shots[0], &shots[1], &shots[2]].map(|s| total(&s[engine]));
            assert!(first < 0.9 * all && second < 0.9 * all, "{name}: a linked lamp lit both spheres ({first:.0}, {second:.0} of {all:.0})");
            assert!(first > 0.05 * all && second > 0.05 * all, "{name}: a linked lamp lit nothing ({first:.0}, {second:.0} of {all:.0})");
            assert!((first + second - all).abs() < 0.12 * all, "{name}: the two halves come to {:.0}, not {all:.0}", first + second);
        }
    }

    /// A light's shadow links keep its shadow to some parts, in both
    /// renderers alike: a lamp beside two spheres, the near one in line
    /// between it and the far one, leaves the far one dark - until its
    /// shadow links leave the near one out, when its light gets through.
    /// Kept to the near one, nothing changes. Skipped where Cycles is not
    /// installed or predates shadow linking.
    #[test]
    fn a_shadow_linked_part_casts_no_shadow() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::Environment;
        use tequila_render_core::light::{Light, LightKind, LightLinks, LightSet, PointLight};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let shadowed = |blockers: LightLinks| {
            move |scene: &mut tequila_render_core::scene::SceneSnapshot| {
                // Side by side across the view, the lamp far off to the left
                // in line with both.
                let right = vec3(3.0, 0.0, -2.0).normalize();
                let mut second = scene.instances[0].clone();
                second.id = tequila_core::ids::InstanceId(1);
                second.transform = tequila_core::math::Transform::translation(right * 0.65) * second.transform;
                scene.instances[0].transform = tequila_core::math::Transform::translation(-right * 0.65) * scene.instances[0].transform;
                scene.instances.push(second);
                scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
                let mut lamp = Light::new(LightKind::Point(PointLight { position: -right * 5.0 + vec3(0.0, 0.5, 0.0), intensity: Spectrum::splat(25.0), radius: 0.02 }));
                lamp.shadow_links = blockers.clone();
                *scene = scene.clone().with_lights(Arc::new(LightSet::new(vec![lamp])));
            }
        };
        let total = |image: &[u8]| image.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>();
        let shots = [
            both_engines("shadow-linked-all", shadowed(LightLinks::All)),
            both_engines("shadow-linked-far", shadowed(LightLinks::Only(Arc::from([1u32])))),
            both_engines("shadow-linked-near", shadowed(LightLinks::AllBut(Arc::from([1u32])))),
        ];
        for (engine, name) in [(0, "Tequila's renderer"), (1, "Cycles")] {
            let [all, far, near] = [&shots[0], &shots[1], &shots[2]].map(|s| total(&s[engine]));
            assert!(far > 1.3 * all, "{name}: the near sphere still shadowed the far one ({far:.0} against {all:.0})");
            assert!((near - all).abs() < 0.05 * all, "{name}: kept to the near sphere, the shadow changed ({near:.0} against {all:.0})");
        }
    }

    /// A spot's falloff exponent dims its penumbra in Cycles as in Tequila's
    /// renderer: a spot whose edge crosses the sphere lights it
    /// alike in both at a falloff of one and of three, and less at three.
    /// Cycles dropped the exponent, and measured the blend in angles where
    /// Tequila measures cosines. Skipped where Cycles is not installed.
    #[test]
    fn a_spots_falloff_reaches_cycles() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::Environment;
        use tequila_render_core::light::{Light, LightKind, LightSet, SpotLight};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let spot = |falloff: f32| {
            move |scene: &mut tequila_render_core::scene::SceneSnapshot| {
                scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
                let lamp = Light::new(LightKind::Spot(SpotLight {
                    // From beside the camera, the penumbra across the face
                    // it sees.
                    position: vec3(1.6, 2.6, 2.4),
                    direction: (vec3(0.0, 1.0, 0.0) - vec3(1.6, 2.6, 2.4)).normalize(),
                    intensity: Spectrum::splat(40.0),
                    inner_angle: 0.18,
                    outer_angle: 0.26,
                    falloff,
                }));
                *scene = scene.clone().with_lights(Arc::new(LightSet::new(vec![lamp])));
            }
        };
        let total = |image: &[u8]| image.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>();
        let soft = both_engines("spot-falloff-1", spot(1.0));
        let steep = both_engines("spot-falloff-3", spot(3.0));
        let (soft, steep) = (soft.map(|i| total(&i)), steep.map(|i| total(&i)));
        eprintln!("falloff 1: Tequila {:.0}, Cycles {:.0}; falloff 3: Tequila {:.0}, Cycles {:.0}", soft[0], soft[1], steep[0], steep[1]);
        for (name, [tequila, cycles]) in [("one", soft), ("three", steep)] {
            assert!((cycles / tequila - 1.0).abs() < 0.12, "at a falloff of {name}, Cycles lit {cycles:.0} where Tequila lit {tequila:.0}");
        }
        assert!(steep[0] < 0.95 * soft[0] && steep[1] < 0.95 * soft[1], "a steeper falloff dimmed nothing: {soft:?} to {steep:?}");
    }

    /// A glowing part lights its neighbour in both renderers while Light
    /// source materials is on, and with it off lights nothing, though it
    /// still glows: beside a glowing sphere, a grey one is lit alike in
    /// both, and dark in both once it is off. The setting reached neither
    /// renderer. Skipped where Cycles is not installed.
    #[test]
    fn light_source_materials_light_their_neighbours_or_not() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::Environment;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let beside = |scene: &mut tequila_render_core::scene::SceneSnapshot| {
            let right = vec3(3.0, 0.0, -2.0).normalize() * 1.1;
            let mut lamp = scene.instances[0].clone();
            lamp.id = tequila_core::ids::InstanceId(1);
            lamp.material = tequila_core::ids::MaterialId(1);
            lamp.transform = tequila_core::math::Transform::translation(right) * lamp.transform;
            scene.instances[0].transform = tequila_core::math::Transform::translation(-right) * scene.instances[0].transform;
            scene.instances.push(lamp);
            let mut glow = tequila_render_core::material::Material::default();
            glow.base_color = Spectrum::ZERO;
            glow.emission_luminance = 2.0;
            // Matte, so what it shows is the light it gets, not the glow's
            // reflection, which stays either way.
            scene.materials.truncate(1);
            scene.materials[0].specular_weight = 0.0;
            scene.materials[0].coat_weight = 0.0;
            scene.materials.push(glow);
            scene.programs.clear();
            scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
        };
        // The grey sphere's side of the picture, clear of the glowing one.
        let lit = |image: &[u8]| {
            image.chunks_exact(4).enumerate().filter(|(i, _)| i % 96 < 36).map(|(_, p)| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>()
        };
        let glowing = |image: &[u8]| {
            image.chunks_exact(4).enumerate().filter(|(i, _)| i % 96 >= 60).map(|(_, p)| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>()
        };
        // With firefly rejection, as a render has it: both renderers sample
        // the glow as a light, so no rare, heavy hit is left to clip.
        let lighting = |light_source_materials: bool| Lit { config: IntegratorConfig { light_source_materials, ..IntegratorConfig::product() }, reject_fireflies: true, camera: None };
        let on = both_engines_lit("light-materials-on", 1.0, lighting(true), beside);
        let off = both_engines_lit("light-materials-off", 1.0, lighting(false), beside);
        let (on_lit, off_lit) = (on.clone().map(|i| lit(&i)), off.clone().map(|i| lit(&i)));
        eprintln!("the grey sphere: on {on_lit:?}, off {off_lit:?}");
        for (engine, name) in [(0, "Tequila's renderer"), (1, "Cycles")] {
            assert!(off_lit[engine] < 0.2 * on_lit[engine], "{name}: with light source materials off the glow still lit its neighbour ({:.0} of {:.0})", off_lit[engine], on_lit[engine]);
            assert!(glowing(&off[engine]) > 0.8 * glowing(&on[engine]), "{name}: with light source materials off the glow went out");
        }
        assert!((on_lit[1] / on_lit[0] - 1.0).abs() < 0.03, "the glow lit its neighbour {:.0} in Tequila's renderer, {:.0} in Cycles", on_lit[0], on_lit[1]);
    }

    /// A sun lights alike in Cycles and Tequila's renderer, sharp or with an
    /// angle, and its angle softens without dimming: a sun from over the
    /// camera's shoulder lights the sphere the same in both, at no angle and
    /// at eight degrees, and about as much at either. Skipped where Cycles is
    /// not installed.
    #[test]
    fn a_sun_lights_alike_in_both_renderers() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::Environment;
        use tequila_render_core::light::{DistantLight, Light, LightKind, LightSet};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let sun = |degrees: f32| {
            move |scene: &mut tequila_render_core::scene::SceneSnapshot| {
                scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
                let sun = Light::new(LightKind::Distant(DistantLight {
                    direction: vec3(-0.5, -1.0, -0.8).normalize(),
                    irradiance: Spectrum::splat(1.5),
                    half_angle: (degrees * 0.5).to_radians(),
                }));
                *scene = scene.clone().with_lights(Arc::new(LightSet::new(vec![sun])));
            }
        };
        let total = |image: &[u8]| image.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>();
        let sharp = both_engines("sun-sharp", sun(0.0)).map(|i| total(&i));
        let soft = both_engines("sun-soft", sun(8.0)).map(|i| total(&i));
        eprintln!("sharp: Tequila {:.0}, Cycles {:.0}; soft: Tequila {:.0}, Cycles {:.0}", sharp[0], sharp[1], soft[0], soft[1]);
        for (name, [tequila, cycles]) in [("sharp", sharp), ("eight degrees", soft)] {
            assert!(tequila > 1000.0, "the {name} sun lit nothing");
            assert!((cycles / tequila - 1.0).abs() < 0.1, "the {name} sun lit {cycles:.0} in Cycles, {tequila:.0} in Tequila's renderer");
        }
        for engine in 0..2 {
            assert!((soft[engine] / sharp[engine] - 1.0).abs() < 0.05, "an angle changed how much the sun lights: {sharp:?} to {soft:?}");
        }
    }

    /// A softbox's spread gathers its light in Cycles as in Tequila's
    /// renderer: aimed at the sphere, a panel lights it alike in both, and
    /// behind a forty-degree grid more brightly, alike again. Cycles had no
    /// spread from Tequila, which had none of its own. Skipped where Cycles
    /// is not installed.
    #[test]
    fn a_softboxs_spread_gathers_its_light_in_both_renderers() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::Environment;
        use tequila_render_core::light::{AreaLight, Light, LightKind, LightSet};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let softbox = |degrees: f32| {
            move |scene: &mut tequila_render_core::scene::SceneSnapshot| {
                scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
                let position = vec3(1.6, 2.6, 2.4);
                let facing = (vec3(0.0, 1.0, 0.0) - position).normalize();
                let half_u = Vec3::Y.cross(facing).normalize() * 0.3;
                let half_v = facing.cross(half_u).normalize() * 0.3;
                let panel = Light::new(LightKind::Area(AreaLight {
                    position,
                    half_u,
                    half_v,
                    radiance: Spectrum::splat(4.0),
                    disc: false,
                    two_sided: false,
                    spread: degrees.to_radians(),
                }));
                *scene = scene.clone().with_lights(Arc::new(LightSet::new(vec![panel])));
            }
        };
        let total = |image: &[u8]| image.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>();
        let plain = both_engines("softbox-plain", softbox(180.0)).map(|i| total(&i));
        let gridded = both_engines("softbox-grid", softbox(40.0)).map(|i| total(&i));
        eprintln!("plain: Tequila {:.0}, Cycles {:.0}; forty degrees: Tequila {:.0}, Cycles {:.0}", plain[0], plain[1], gridded[0], gridded[1]);
        for (name, [tequila, cycles]) in [("plain", plain), ("gridded", gridded)] {
            assert!(tequila > 1000.0, "the {name} softbox lit nothing");
            assert!((cycles / tequila - 1.0).abs() < 0.1, "the {name} softbox lit {cycles:.0} in Cycles, {tequila:.0} in Tequila's renderer");
        }
        assert!(gridded[0] > 1.2 * plain[0] && gridded[1] > 1.2 * plain[1], "a grid gathered no light into its beam: {plain:?} to {gridded:?}");
    }

    /// Orthographic, shifted and panoramic cameras frame the sphere in Cycles
    /// where Tequila's renderer does: the box round its lit pixels is the
    /// same in both, within a pixel or two, through each. Cycles rendered
    /// every camera as plain perspective. Skipped where Cycles is not
    /// installed.
    #[test]
    fn cameras_frame_alike_in_both_renderers() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::camera::{Camera, Projection};
        use tequila_render_core::environment::Environment;
        use tequila_render_core::light::{Light, LightKind, LightSet, PointLight};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let lit = |scene: &mut tequila_render_core::scene::SceneSnapshot| {
            scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
            let lamp = Light::new(LightKind::Point(PointLight { position: vec3(2.0, 4.0, 3.0), intensity: Spectrum::splat(30.0), radius: 0.0 }));
            *scene = scene.clone().with_lights(Arc::new(LightSet::new(vec![lamp])));
        };
        // Where the lit pixels are: left, right, top, bottom.
        let framed = |image: &[u8]| {
            let (mut left, mut right, mut top, mut bottom) = (96usize, 0usize, 72usize, 0usize);
            for (i, p) in image.chunks_exact(4).enumerate() {
                if u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) > 60 {
                    let (x, y) = (i % 96, i / 96);
                    (left, right, top, bottom) = (left.min(x), right.max(x), top.min(y), bottom.max(y));
                }
            }
            [left, right, top, bottom]
        };
        let looking = Camera::look_at(vec3(4.0, 2.5, 6.0), vec3(0.0, 1.0, 0.0), Vec3::Y);
        let mut wrong = Vec::new();
        for (name, projection) in [
            ("orthographic", Projection::Orthographic { scale: 3.0 }),
            ("shifted", Projection::Shift { focal_length_mm: 60.0, shift_x: 0.2, shift_y: -0.15 }),
            ("panoramic", Projection::Equirectangular),
        ] {
            let camera = Camera { projection, ..looking };
            let lit_by = Lit { config: IntegratorConfig::product(), reject_fireflies: true, camera: Some(camera) };
            let [tequila, cycles] = both_engines_lit(&format!("camera-{name}"), 1.0, lit_by, lit);
            let (a, b) = (framed(&tequila), framed(&cycles));
            eprintln!("{name}: Tequila {a:?}, Cycles {b:?}");
            assert!(a[1] > a[0] && a[3] > a[2], "{name}: Tequila's renderer shows no sphere");
            if a.iter().zip(&b).any(|(x, y)| x.abs_diff(*y) > 2) {
                wrong.push(format!("{name}: Tequila {a:?}, Cycles {b:?}"));
            }
        }
        assert!(wrong.is_empty(), "framed differently: {wrong:#?}");

        // And a panorama the right way round: a sky bright in one quarter
        // only, and higher up than down, lands on the same columns and rows
        // of both pictures.
        use tequila_texture::image::{ColorSpace, Image};
        let (w, h) = (64, 32);
        let mut data = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let v = if x < w / 4 && y < h / 3 { 1.0 } else { 0.0 };
                data.extend_from_slice(&[v, v, v]);
            }
        }
        let sky = Arc::new(Environment::new(Arc::new(Image { width: w, height: h, channels: 3, color_space: ColorSpace::LinearSrgb, data }), 1.0));
        let camera = Camera { projection: Projection::Equirectangular, ..looking };
        let lit_by = Lit { config: IntegratorConfig::product(), reject_fireflies: true, camera: Some(camera) };
        let [tequila, cycles] = both_engines_lit("camera-panorama-sky", 1.0, lit_by, move |scene| {
            scene.instances.clear();
            scene.environment = sky.clone();
        });
        // Where the light is, weighted by it, and how much of the picture is
        // bright: a stray faint pixel moves neither.
        let centre = |image: &[u8]| {
            let (mut x, mut y, mut sum, mut bright) = (0.0, 0.0, 0.0, 0usize);
            for (i, p) in image.chunks_exact(4).enumerate() {
                let v = f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]);
                (x, y, sum) = (x + v * (i % 96) as f64, y + v * (i / 96) as f64, sum + v);
                bright += usize::from(v > 384.0);
            }
            (x / sum.max(1.0), y / sum.max(1.0), bright)
        };
        let (a, b) = (centre(&tequila), centre(&cycles));
        eprintln!("panoramic sky: Tequila {a:?}, Cycles {b:?}");
        assert!(a.2 > 50, "Tequila's panorama shows no sky");
        assert!((a.0 - b.0).abs() < 1.5 && (a.1 - b.1).abs() < 1.5 && a.2.abs_diff(b.2) * 10 < a.2, "the panorama's sky is elsewhere: Tequila {a:?}, Cycles {b:?}");
    }

    /// A Cycles render asked for passes keeps its engine, and its passes are
    /// Tequila's: the depth and normal at each pixel are what Tequila's own
    /// tracer finds along the same camera ray, and the light groups add up
    /// to the picture. Cycles left passes and light groups to Tequila's
    /// renderer, and said so. Skipped where Cycles is not installed or
    /// predates passes.
    #[test]
    fn cycles_keeps_passes_tequila_would() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::intersect::Intersector;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        if !tequila_render_cycles::has_passes() {
            eprintln!("skipped: this Cycles library predates passes");
            return;
        }
        let directory = scratch("tequila-cycles-passes");
        let (width, height) = (48, 36);
        let settings = RenderSettings {
            engine: RenderEngine::CyclesCpu,
            width,
            height,
            max_samples: 16,
            denoise: false,
            write_passes: true,
            relight: true,
            format: tequila_scene::render::OutputFormat::Exr,
            ..Default::default()
        };
        assert!(settings.effective_write_passes());
        let mut job = job(1, &directory, settings);
        // A lamp on a layer of its own, beside the environment: two groups.
        let lamp = tequila_render_core::light::Light::new(tequila_render_core::light::LightKind::Point(tequila_render_core::light::PointLight {
            position: vec3(1.5, 3.0, 2.0),
            intensity: Spectrum::splat(8.0),
            radius: 0.0,
        }))
        .with_layer(2);
        job.scene = Arc::new((*job.scene).clone().with_lights(Arc::new(tequila_render_core::light::LightSet::new(vec![lamp]))));
        job.object_names = vec!["Sphere".into(); job.scene.instances.len()];
        job.material_names = vec!["Plastic".into(); job.scene.materials.len()];
        assert_eq!(engine_for(&job.settings, false).0, RenderEngine::CyclesCpu, "passes sent the render to Tequila's renderer");
        let (events, _heard) = std::sync::mpsc::channel();
        let frame = render_cycles(&job, RenderEngine::CyclesCpu, &events, &AtomicBool::new(false)).expect("Cycles renders");
        let passes = frame.passes.as_ref().expect("the passes");

        // Along the same rays, through Tequila's own tracer.
        let device = tequila_render_cpu::Device::shared().unwrap();
        let intersector = tequila_render_cpu::EmbreeIntersector::build(&device, (*job.scene).clone()).unwrap();
        let aspect = width as f32 / height as f32;
        let (mut both, mut near, mut facing, mut agree) = (0, 0, 0, 0);
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                let ray = job.camera.pick_ray(tequila_core::math::vec2((x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32), aspect);
                let hit = intersector.intersect(&ray);
                let seen = passes.depth[i].is_finite();
                agree += usize::from(seen == hit.is_some());
                if let (Some(hit), true) = (hit, seen) {
                    both += 1;
                    near += usize::from((passes.depth[i] - hit.t).abs() < 0.02 * hit.t);
                    // Averaged over the pixel's samples, shorter at an edge.
                    facing += usize::from(passes.normal[i].try_normalize().is_some_and(|n| n.dot(hit.shading_normal) > 0.95));
                }
            }
        }
        eprintln!("{both} pixels hit in both; depth within 2% at {near}, normal within 18 degrees at {facing}; hit or missed alike at {agree} of {}", width * height);
        assert!(both > 300, "the sphere was not in both");
        assert!(agree * 100 >= width * height * 97, "Cycles' depth saw the sphere elsewhere");
        assert!(near * 100 >= both * 95 && facing * 100 >= both * 95, "the depth or the normal is not Tequila's");

        // The light groups add up to the picture, before Tequila lays
        // anything over it.
        let summed: Vec<Spectrum> = (0..width * height).map(|i| passes.groups.iter().fold(Spectrum::ZERO, |sum, (_, image)| sum + image[i])).collect();
        let (mut difference, mut total) = (0.0f64, 0.0f64);
        for (sum, beauty) in summed.iter().zip(&frame.color) {
            difference += f64::from((sum.luminance() - beauty.luminance()).abs());
            total += f64::from(beauty.luminance().abs());
        }
        eprintln!("light groups {:?}: off the picture by {:.4} of it", passes.groups.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), difference / total.max(1e-9));
        assert!(difference < 0.02 * total, "the light groups do not add up to the picture");
        let names: Vec<&str> = passes.groups.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["LightLayer2", "Environment"], "the light groups are not Tequila's");
        let lamp_lit: f64 = passes.groups[0].1.iter().map(|c| f64::from(c.luminance())).sum();
        assert!(lamp_lit > 0.05 * total, "the lamp's group holds none of its light");
        assert!(passes.mattes.len() == 2 && passes.mattes[0].manifest().iter().any(|e| e.name == "Sphere"), "no object Cryptomatte named the sphere");
        assert!(passes.mattes[0].coverage_of("Sphere", width / 2, height / 2) > 0.9, "the object Cryptomatte did not cover the sphere");

        // And all the way through the queue: an EXR written, and the groups
        // kept to relight with.
        let renderer = BatchRenderer::new();
        renderer.submit(job);
        match wait_for_finish(&renderer) {
            BatchEvent::Finished { path, relight, .. } => {
                assert!(path.ends_with(".exr") && std::fs::metadata(&path).is_ok_and(|m| m.len() > 1000), "no EXR at {path}");
                let relight = relight.expect("the light groups, kept");
                let kept: Vec<&str> = relight.groups.iter().map(|(n, _)| n.as_str()).collect();
                assert!(kept.starts_with(&["LightLayer2", "Environment"]), "{kept:?}");
            }
            BatchEvent::Failed { message, .. } => panic!("{message}"),
            _ => unreachable!(),
        }
    }

    /// Curvature and Occlusion nodes shade alike in Cycles and Tequila's
    /// renderer: with the sphere's vertices given a curvature and an
    /// occlusion that run from its foot to its top, each node's mask lies
    /// over the sphere the same way in both, at the same levels. Cycles read
    /// both as a flat half. Skipped where Cycles is not installed.
    #[test]
    fn curvature_and_occlusion_reach_cycles() {
        use tequila_materials::graph::{CurvatureMode, MaterialGraph, Node, NodeKind};
        use tequila_materials::material_type::MaterialType;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let shaped = |kind: NodeKind| {
            move |scene: &mut tequila_render_core::scene::SceneSnapshot| {
                let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
                let node = graph.add_node(Node::new(kind.clone()));
                let solo = graph.soloed_at(node, 0).expect("a value to show");
                let program = solo.compile().expect("compiles");
                *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                    .with_programs(vec![Some(Arc::new(program))], Arc::new(tequila_render_core::shade::TextureTable::new()));
                scene.materials[0] = solo.material().clone();
                // From the foot to the top: a curvature from -40 to 40 in
                // the sphere's units, and an occlusion from closed to open.
                let mut mesh = (*scene.meshes[0]).clone();
                mesh.curvature = mesh.positions.iter().map(|p| 40.0 * p.y).collect();
                mesh.occlusion = mesh.positions.iter().map(|p| (0.5 - 0.5 * p.y).clamp(0.0, 1.0)).collect();
                scene.meshes[0] = Arc::new(mesh);
            }
        };
        let mut wrong = Vec::new();
        for (name, kind) in [
            ("curvature-both", NodeKind::Curvature { mode: CurvatureMode::Both, radius: 0.02, contrast: 1.5 }),
            ("curvature-convex", NodeKind::Curvature { mode: CurvatureMode::Convex, radius: 0.03, contrast: 1.0 }),
            ("occlusion", NodeKind::Occlusion { radius: 0.01, contrast: 2.0 }),
        ] {
            let [tequila, cycles] = both_engines(&format!("shape-{name}"), shaped(kind));
            let (a, b) = (inside_the_sphere(&tequila), inside_the_sphere(&cycles));
            let ((ma, sa), (mb, sb)) = (mean_and_spread(&a), mean_and_spread(&b));
            let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
            for (x, y) in a.iter().zip(&b) {
                cov += (x - ma) * (y - mb);
                va += (x - ma) * (x - ma);
                vb += (y - mb) * (y - mb);
            }
            let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
            eprintln!("{name}: correlation {correlation:.3}, Tequila {ma:.1} spread {sa:.1}, Cycles {mb:.1} spread {sb:.1}");
            if sa < 10.0 || correlation < 0.97 || (ma - mb).abs() > 6.0 + 0.05 * ma {
                wrong.push(format!("{name}: correlation {correlation:.3}, Tequila {ma:.1} spread {sa:.1}, Cycles {mb:.1} spread {sb:.1}"));
            }
        }
        assert!(wrong.is_empty(), "these shade differently: {wrong:#?}");
    }

    /// A plastic mixed half and half with a metal shades alike in both
    /// renderers - Tequila's sees one or the other at each bounce, Cycles
    /// mixes its two closures - and is neither surface alone; an Add Shader
    /// of a glow lights the same surface up alike in both. Cycles had no
    /// way to mix two surfaces. Skipped where Cycles is not installed.
    #[test]
    fn mixed_shaders_shade_alike_in_both_renderers() {
        use tequila_core::spectrum::Spectrum;
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, Value, LAYER_INPUT};
        use tequila_materials::material_type::MaterialType;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let red = tequila_render_core::material::Material { base_color: Spectrum { r: 0.8, g: 0.05, b: 0.05 }, ..MaterialType::Plastic.defaults() };
        let gold = tequila_render_core::material::Material { base_metalness: 1.0, base_color: Spectrum { r: 1.0, g: 0.8, b: 0.35 }, specular_roughness: 0.25, ..MaterialType::Plastic.defaults() };
        let glow = tequila_render_core::material::Material { emission_luminance: 1.5, emission_color: Spectrum { r: 0.2, g: 0.4, b: 1.0 }, base_color: Spectrum::ZERO, ..MaterialType::Plastic.defaults() };
        let with = |a: tequila_render_core::material::Material, b: Option<(tequila_render_core::material::Material, f32, bool)>| {
            move |scene: &mut tequila_render_core::scene::SceneSnapshot| {
                let mut graph = MaterialGraph::from_material(a.clone());
                if let Some((b, factor, add)) = b.clone() {
                    let shader = graph.add_node(Node::new(NodeKind::Shader(Box::new(b))));
                    let mut mix = Node::new(NodeKind::MixShader { add });
                    mix.literals.insert(Socket::Input(0), Value::Float(factor));
                    let mix = graph.add_node(mix);
                    graph.connect(shader, mix, Socket::Input(1));
                    graph.connect(mix, graph.output(), LAYER_INPUT);
                }
                let program = graph.compile().expect("compiles");
                *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                    .with_programs(vec![Some(Arc::new(program))], Arc::new(tequila_render_core::shade::TextureTable::new()));
                scene.materials[0] = graph.material().clone();
            }
        };
        let shots = [
            ("red", both_engines("mix-red", with(red.clone(), None))),
            ("gold", both_engines("mix-gold", with(gold.clone(), None))),
            ("half and half", both_engines("mix-half", with(red.clone(), Some((gold.clone(), 0.5, false))))),
            ("plus a glow", both_engines("mix-glow", with(red.clone(), Some((glow.clone(), 0.0, true))))),
        ];
        let means: Vec<(&str, [[f64; 3]; 2])> = shots.iter().map(|(name, [t, c])| (*name, [channel_means(t), channel_means(c)])).collect();
        for (name, [t, c]) in &means {
            eprintln!("{name}: Tequila {t:.1?}, Cycles {c:.1?}");
            for k in 0..3 {
                assert!((t[k] - c[k]).abs() < 6.0 + 0.06 * t[k].max(c[k]), "{name}: Tequila {t:.1?}, Cycles {c:.1?}");
            }
        }
        let [red, gold, half, glowing] = [&means[0].1, &means[1].1, &means[2].1, &means[3].1];
        for engine in 0..2 {
            // Between the two in each channel, as a mix is.
            for k in 0..3 {
                let (low, high) = (red[engine][k].min(gold[engine][k]), red[engine][k].max(gold[engine][k]));
                assert!(half[engine][k] > low - 3.0 && half[engine][k] < high + 3.0, "the mix is outside its two surfaces");
            }
            assert!(glowing[engine][2] > red[engine][2] + 10.0, "the added glow lit nothing");
        }
    }

    /// A flat label kept to one side prints on the faces looking that way and
    /// not on those behind, in both renderers alike: projected straight
    /// through a sphere along each axis in turn, where the camera looks it
    /// prints for at most one of the two sides - for one of them on some
    /// axis - and on both sides wherever either does (and also on faces too
    /// far off the axis for one side). Skipped where Cycles is not installed.
    #[test]
    fn a_one_sided_label_prints_on_one_side_only() {
        use tequila_core::spectrum::Spectrum;
        use tequila_materials::decal::DecalSide;
        use tequila_materials::graph::{MaterialGraph, TextureId};
        use tequila_materials::material_type::MaterialType;
        use tequila_texture::image::{ColorSpace, Image};
        use tequila_texture::projection::{Axis, Mapping, Projection};
        use tequila_texture::texture::{MapRole, Texture};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let middle = |image: &[u8]| -> f64 {
            let (mut sum, mut n) = (0.0, 0.0);
            for y in 30..38 {
                for x in 44..52 {
                    sum += f64::from(image[(y * 96 + x) * 4 + 1]);
                    n += 1.0;
                }
            }
            sum / n
        };
        let printed = |axis: Axis, side: Option<DecalSide>| {
            move |scene: &mut tequila_render_core::scene::SceneSnapshot| {
                let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
                graph.material_mut().base_color = Spectrum::splat(0.05);
                // The whole sphere under the artwork.
                if let Some(side) = side {
                    let mut print = tequila_materials::decal::Decal::new("Print", TextureId(1));
                    print.mapping = Mapping { projection: Projection::Planar, axis, scale: tequila_core::math::vec3(0.01, 0.01, 0.01), offset: tequila_core::math::vec2(0.5, 0.5), real_world_units: false, ..Mapping::default() };
                    print.side = side;
                    graph.decals.push(print);
                }
                let program = graph.compile().expect("compiles");
                let mut table = tequila_render_core::shade::TextureTable::new();
                table.insert(TextureId(1), Arc::new(Texture::from_linear(
                    Image { width: 4, height: 4, channels: 4, color_space: ColorSpace::LinearSrgb, data: [0.9, 0.9, 0.9, 1.0].repeat(16) },
                    MapRole::Color,
                    Mapping::default(),
                )));
                scene.materials[0] = graph.material().clone();
                *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                    .with_programs(vec![Some(Arc::new(program))], Arc::new(table));
            }
        };
        let [bare, _] = both_engines("label-side-none", printed(Axis::Z, None));
        let bare = middle(&bare);
        let mut one_sided = false;
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            let shot = |side: DecalSide| {
                let [tequila, cycles] = both_engines(&format!("label-side-{axis:?}-{}", side.key()), printed(axis, Some(side)));
                let (tequila, cycles) = (middle(&tequila), middle(&cycles));
                assert!((tequila - cycles).abs() < 10.0, "{axis:?} {side:?}: {tequila:.1} in Tequila's render, {cycles:.1} in Cycles'");
                tequila > bare + 20.0
            };
            let (plus, minus, both) = (shot(DecalSide::Positive), shot(DecalSide::Negative), shot(DecalSide::Both));
            assert!(!(plus && minus), "along {axis:?} it prints for both sides");
            assert!(both || !(plus || minus), "along {axis:?}: + {plus}, - {minus}, both sides {both}");
            one_sided |= plus || minus;
        }
        assert!(one_sided, "no axis printed for either side");
    }

    /// Coloured glass with a depth absorbs as it goes, in Cycles as in
    /// Tequila's renderer: through the middle of a ball of amber liquid -
    /// two depths of it - the sky behind comes out the same colour in both.
    /// Cycles tinted the surface instead, once, whatever the thickness.
    /// Skipped where Cycles is not installed.
    #[test]
    fn a_liquid_absorbs_by_its_depth() {
        use tequila_core::spectrum::Spectrum;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let liquid = |scene: &mut tequila_render_core::scene::SceneSnapshot| {
            let m = &mut scene.materials[0];
            m.transmission_weight = 1.0;
            m.transmission_color = Spectrum { r: 0.8, g: 0.45, b: 0.15 };
            m.transmission_depth = 1.0;
            // Index-matched: no interface, so all that differs is the
            // absorption along the way.
            m.specular_ior = 1.0;
            m.specular_roughness = 0.0;
            m.base_color = Spectrum::ONE;
            scene.programs.clear();
        };
        let [tequila, cycles] = both_engines("liquid", liquid);
        let middle = |image: &[u8]| -> [f64; 3] {
            let (mut sum, mut n) = ([0.0; 3], 0.0);
            // Where the camera looks through the sphere's middle.
            for y in 30..38 {
                for x in 44..52 {
                    let i = (y * 96 + x) * 4;
                    for c in 0..3 {
                        sum[c] += f64::from(image[i + c]);
                    }
                    n += 1.0;
                }
            }
            sum.map(|s| s / n)
        };
        let (a, b) = (middle(&tequila), middle(&cycles));
        assert!(a[0] > 40.0, "Tequila's liquid came out black: {a:.1?}");
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() < 12.0, "through the liquid Tequila shows {a:.1?}, Cycles {b:.1?}");
        }
    }

    /// The demo sphere's material as `texture` read tri-planar - in
    /// real-world units, `size` across - into `param`: a texture pack's
    /// mapping.
    fn tri_planar_material(
        scene: &mut tequila_render_core::scene::SceneSnapshot,
        param: tequila_materials::material_type::Param,
        role: tequila_texture::texture::MapRole,
        texture: tequila_texture::image::Image,
        size: f32,
    ) {
        use tequila_core::spectrum::Spectrum;
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, TextureId, Value};
        use tequila_materials::material_type::MaterialType;
        use tequila_texture::projection::{Mapping, Projection};
        use tequila_texture::texture::Texture;
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        graph.material_mut().base_color = Spectrum::splat(0.8);
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping {
                projection: Projection::TriPlanar,
                real_world_units: true,
                scale: tequila_core::math::vec3(size, size, 1.0),
                ..Mapping::default()
            },
        }));
        let image = graph.add_node(Node::new(NodeKind::ImageTexture { texture: TextureId(1), fallback: Value::Float(0.0) }));
        graph.connect(mapping, image, Socket::Input(0));
        let output = graph.output();
        graph.connect(image, output, Socket::Param(param));
        let program = graph.compile().expect("compiles");
        let mut table = tequila_render_core::shade::TextureTable::new();
        table.insert(TextureId(1), Arc::new(Texture::from_linear(texture, role, tequila_texture::projection::Mapping::default())));
        scene.materials[0] = graph.material().clone();
        *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
            .with_programs(vec![Some(Arc::new(program))], Arc::new(table));
    }

    /// A texture pack's tri-planar mapping lands its picture where Tequila's
    /// renderer does: a red and white checker, real-world sized on a sphere
    /// scaled to half, covers the same pixels red in both. Cycles read every
    /// tri-planar texture through the mesh's UVs, and no size as a size in
    /// the scene. The squares are 8 by 8 texels so their edges stay sharp
    /// and the test measures placement, not filtering: Cycles blends an
    /// 8-bit colour texture on the sRGB curve (decoded in the kernel), as
    /// Blender does, where Tequila blends in linear, and a checker of four
    /// texels was one blend from red to white. Skipped where Cycles is not
    /// installed.
    #[test]
    fn a_tri_planar_texture_lands_where_tequila_puts_it() {
        use tequila_texture::image::{ColorSpace, Image};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let checker = Image {
            width: 16,
            height: 16,
            channels: 3,
            color_space: ColorSpace::LinearSrgb,
            data: (0..256).flat_map(|i| if (i % 16 < 8) == (i / 16 < 8) { [1.0, 0.0, 0.0] } else { [1.0; 3] }).collect(),
        };
        let [tequila, cycles] = both_engines("tri-planar", |scene| {
            tri_planar_material(scene, tequila_materials::material_type::Param::BaseColor, tequila_texture::texture::MapRole::Color, checker.clone(), 0.35);
            // Half size: a real-world size counts in the scene's units, so the
            // squares stay the same size on a smaller sphere - fewer of them.
            let half = tequila_core::math::Mat4::translation(vec3(0.0, 1.0, 0.0)) * tequila_core::math::Mat4::scale(Vec3::splat(0.5));
            scene.instances[0].transform = tequila_core::math::Transform::from_matrix(half).expect("invertible");
        });
        let red = |image: &[u8]| -> Vec<bool> { image.chunks_exact(4).map(|p| f64::from(p[0]) - f64::from(p[1]) > 40.0).collect() };
        let (a, b) = (red(&tequila), red(&cycles));
        let covered = a.iter().filter(|&&x| x).count();
        assert!(covered > 200, "Tequila's render shows no checker: {covered} red pixels");
        let agree = a.iter().zip(&b).filter(|(&x, &y)| x && y).count();
        let stray = a.iter().zip(&b).filter(|(&x, &y)| !x && y).count();
        assert!(
            agree as f64 > 0.8 * covered as f64 && (stray as f64) < 0.2 * covered as f64,
            "of Tequila's {covered} red pixels Cycles shows {agree} red, and {stray} where Tequila has none"
        );
    }

    /// A texture pack's tri-planar normal map leans the surface the way
    /// Tequila's renderer leans it: a map leaning every texel across its
    /// picture changes the sphere's shading the same way in both. Cycles
    /// decoded it in the frame of the mesh's UVs. Skipped where Cycles is
    /// not installed.
    #[test]
    fn a_tri_planar_normal_map_leans_as_in_tequila() {
        use tequila_texture::image::{ColorSpace, Image};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let map = |texel: [f32; 3]| Image { width: 1, height: 1, channels: 3, color_space: ColorSpace::Raw, data: texel.to_vec() };
        let bumped = |name: &str, texel: [f32; 3]| {
            both_engines(name, move |scene| {
                tri_planar_material(scene, tequila_materials::material_type::Param::Bump, tequila_texture::texture::MapRole::Normal, map(texel), 0.5);
                // The sphere's UVs turned a quarter - u and v swapped - so a
                // map decoded in their frame leans across the way the
                // projection leans it.
                let mut sphere = (*scene.meshes[0]).clone();
                sphere.uvs = sphere.uvs.iter().map(|uv| tequila_core::math::vec2(uv.y, uv.x)).collect();
                scene.meshes[0] = Arc::new(sphere);
                // A lamp beside the sphere, level with it: leaning across the
                // picture - sideways, on the faces in view - turns the surface
                // to it or away, and leaning up or down hardly matters. Under
                // the even sky alone, any lean looks much like another.
                use tequila_render_core::light::{Light, LightKind, LightSet, PointLight};
                let lamp = Light::new(LightKind::Point(PointLight {
                    position: vec3(3.3, 1.0, -2.2),
                    intensity: tequila_core::spectrum::Spectrum::splat(30.0),
                    radius: 0.05,
                }));
                *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                    .with_lights(Arc::new(LightSet::new(vec![lamp])));
            })
        };
        // Leaning one way across the picture and then the other: what any
        // lean does alike - tipping the surface off its normal - cancels,
        // and what is left is the way it leans.
        let [tequila_back, cycles_back] = bumped("normal-back", [0.35, 0.5, 0.97]);
        let [tequila_lean, cycles_lean] = bumped("normal-lean", [0.65, 0.5, 0.97]);
        // Inside the sphere's outline, away from its rim: at a grazing angle
        // Tequila's renderer brightens a leaned surface in a ring of its own
        // (the strong maps that turn it black are the same story), which is
        // nothing to hold Cycles to.
        let camera = Camera::look_at(vec3(2.0, 1.5, 3.0), vec3(0.0, 0.5, 0.0), Vec3::Y);
        let aspect = 96.0 / 72.0;
        let centre = camera.project(vec3(0.0, 1.0, 0.0), aspect).expect("in view");
        let side = (vec3(0.0, 1.0, 0.0) - vec3(2.0, 1.5, 3.0)).cross(Vec3::Y).normalize();
        let edge = camera.project(vec3(0.0, 1.0, 0.0) + side, aspect).expect("in view");
        let radius = ((edge.x - centre.x) * 96.0).hypot((edge.y - centre.y) * 72.0) * 0.7;
        let inside = |i: usize| {
            let (x, y) = ((i % 96) as f32 + 0.5, (i / 96) as f32 + 0.5);
            (x - centre.x * 96.0).hypot(y - centre.y * 72.0) < radius
        };
        let change = |lean: &[u8], back: &[u8]| -> Vec<f64> {
            lean.chunks_exact(4)
                .zip(back.chunks_exact(4))
                .enumerate()
                .filter(|(i, _)| inside(*i))
                .map(|(_, (a, b))| f64::from(a[1]) - f64::from(b[1]))
                .collect()
        };
        let (a, b) = (change(&tequila_lean, &tequila_back), change(&cycles_lean, &cycles_back));
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let (ma, mb) = (mean(&a), mean(&b));
        let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(&b) {
            cov += (x - ma) * (y - mb);
            va += (x - ma) * (x - ma);
            vb += (y - mb) * (y - mb);
        }
        let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
        assert!(va > 500.0, "leaning changed nothing in Tequila's render");
        assert!(correlation > 0.7, "leaning changed the two engines' renders differently: correlation {correlation:.2}");
    }

    /// Metallic paint comes out of Cycles the colour it comes out of
    /// Tequila's renderer: its flakes - silver platelets under the clear
    /// coat, a millimetre across here so they average within a pixel - pale
    /// the green as they do in Tequila's. Cycles dropped the flakes and the
    /// paint came out a flat, deeper green. Skipped where Cycles is not
    /// installed.
    #[test]
    fn metallic_paint_is_the_same_colour_in_cycles() {
        use tequila_core::spectrum::Spectrum;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let [tequila, cycles] = both_engines("paint", |scene| {
            let m = &mut scene.materials[0];
            m.base_color = Spectrum { r: 0.05, g: 0.35, b: 0.05 };
            m.base_metalness = 0.0;
            m.specular_roughness = 0.3;
            m.coat_weight = 1.0;
            m.coat_roughness = 0.02;
            m.flake_weight = 0.6;
            m.flake_color = Spectrum::splat(0.9);
            m.flake_size = 0.001;
            m.flake_spread = 0.3;
            m.flake_roughness = 0.3;
            scene.programs.clear();
        });
        let middle = |image: &[u8]| -> [f64; 3] {
            let (mut sum, mut n) = ([0.0; 3], 0.0);
            for y in 18..30 {
                for x in 40..56 {
                    let i = (y * 96 + x) * 4;
                    for c in 0..3 {
                        sum[c] += f64::from(image[i + c]);
                    }
                    n += 1.0;
                }
            }
            sum.map(|s| s / n)
        };
        let (a, b) = (middle(&tequila), middle(&cycles));
        assert!(a[1] > 40.0, "Tequila's paint came out black: {a:.1?}");
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() < 10.0, "Tequila's paint is {a:.1?}, Cycles' {b:.1?}");
        }
    }

    /// An IES light lights the sphere in Cycles as in Tequila's renderer: a
    /// luminaire throwing most of its light one way round (towards its
    /// right), alone in the dark, leaves the same side lit, as brightly. Cycles
    /// left IES lights out. Skipped where Cycles is not installed.
    #[test]
    fn an_ies_light_lights_as_in_tequila() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::environment::Environment;
        use tequila_render_core::ies::IesProfile;
        use tequila_render_core::light::{IesLight, Light, LightKind, LightSet};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        // Type C: vertical angles from the aim, horizontal from the right.
        // Bright towards the right (0 degrees), dim the other ways.
        let profile = IesProfile::parse(concat!(
            "IESNA:LM-63-2002\n",
            "TILT=NONE\n",
            "1 -1 1 5 5 1 2 0 0 0\n",
            "1 1 0\n",
            "0 45 90 135 180\n",
            "0 90 180 270 360\n",
            "300 300 300 0 0\n",
            "30 30 30 0 0\n",
            "30 30 30 0 0\n",
            "30 30 30 0 0\n",
            "300 300 300 0 0\n",
        ))
        .expect("parses");
        // Above and in front of the sphere, aimed at it, so the side the
        // camera sees is the side it lights; its right level and square to
        // its aim.
        let position = vec3(1.5, 2.8, 2.5);
        let direction = (vec3(0.0, 1.0, 0.0) - position).normalize();
        let right = direction.cross(Vec3::Y).normalize();
        let lamp = Light::new(LightKind::Ies(IesLight {
            position,
            direction,
            right,
            intensity: Spectrum::splat(0.05),
            profile: Arc::new(profile),
        }));
        let [tequila, cycles] = both_engines("ies", move |scene| {
            scene.environment = Arc::new(Environment::uniform(Spectrum::ZERO));
            *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                .with_lights(Arc::new(LightSet::new(vec![lamp.clone()])));
        });
        let brightness = |image: &[u8]| -> Vec<f64> { image.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).collect() };
        let (a, b) = (brightness(&tequila), brightness(&cycles));
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let (ma, mb) = (mean(&a), mean(&b));
        assert!(ma > 10.0, "the IES light left Tequila's render dark: {ma:.1}");
        assert!((mb / ma - 1.0).abs() < 0.2, "the IES light lit Tequila's render to {ma:.1} and Cycles' to {mb:.1}");
        let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(&b) {
            cov += (x - ma) * (y - mb);
            va += (x - ma) * (x - ma);
            vb += (y - mb) * (y - mb);
        }
        let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
        assert!(correlation > 0.9, "the IES light lit the two engines' renders differently: correlation {correlation:.2}");
    }

    /// The sphere's colour from `procedural` read through `gradient`, in
    /// object space, as a material graph wires a patterned colour.
    fn procedural_material(
        scene: &mut tequila_render_core::scene::SceneSnapshot,
        procedural: tequila_texture::procedural::Procedural,
        gradient: tequila_texture::gradient::Gradient,
    ) {
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket};
        use tequila_materials::material_type::{MaterialType, Param};
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let pattern = graph.add_node(Node::new(NodeKind::Procedural { procedural: Box::new(procedural), seed: 7 }));
        let ramp = graph.add_node(Node::new(NodeKind::Gradient { gradient: Box::new(gradient) }));
        graph.connect(pattern, ramp, Socket::Input(0));
        let output = graph.output();
        graph.connect(ramp, output, Socket::Param(Param::BaseColor));
        let program = graph.compile().expect("compiles");
        scene.materials[0] = graph.material().clone();
        *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
            .with_programs(vec![Some(Arc::new(program))], Arc::new(tequila_render_core::shade::TextureTable::new()));
    }

    /// The brightness of each pixel inside the sphere's outline, away from
    /// its rim, in the view `both_engines` renders.
    fn inside_the_sphere(image: &[u8]) -> Vec<f64> {
        use tequila_render_core::camera::Camera;
        let camera = Camera::look_at(vec3(2.0, 1.5, 3.0), vec3(0.0, 0.5, 0.0), Vec3::Y);
        let aspect = 96.0 / 72.0;
        let centre = camera.project(vec3(0.0, 1.0, 0.0), aspect).expect("in view");
        let side = (vec3(0.0, 1.0, 0.0) - vec3(2.0, 1.5, 3.0)).cross(Vec3::Y).normalize();
        let edge = camera.project(vec3(0.0, 1.0, 0.0) + side, aspect).expect("in view");
        let radius = ((edge.x - centre.x) * 96.0).hypot((edge.y - centre.y) * 72.0) * 0.8;
        image
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, _)| ((*i % 96) as f32 + 0.5 - centre.x * 96.0).hypot((*i / 96) as f32 + 0.5 - centre.y * 72.0) < radius)
            .map(|(_, p)| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]))
            .collect()
    }

    /// The mean of each channel inside the sphere's outline.
    fn channel_means(image: &[u8]) -> [f64; 3] {
        let mut means = [0.0; 3];
        for (c, mean) in means.iter_mut().enumerate() {
            let one: Vec<u8> = image.chunks_exact(4).flat_map(|p| [p[c], 0, 0, 255]).collect();
            let v = inside_the_sphere(&one);
            *mean = v.iter().sum::<f64>() / v.len() as f64;
        }
        means
    }

    fn mean_and_spread(v: &[f64]) -> (f64, f64) {
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        (mean, (v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / v.len() as f64).sqrt())
    }

    /// Procedural wood comes out of Cycles with its rings where Tequila's
    /// renderer puts them: rings about the depth axis, barely wandering, read
    /// through a ramp with a soft stretch and a hard edge, light the same
    /// pixels in both, in the same browns. Cycles read every procedural as a
    /// flat grey, and every ramp as the grey it was handed. Skipped where Cycles is not installed.
    #[test]
    fn procedural_wood_has_its_rings_in_cycles() {
        use tequila_core::spectrum::Spectrum;
        use tequila_texture::gradient::{Gradient, Stop};
        use tequila_texture::procedural::Procedural;
        use tequila_texture::projection::Axis;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let [tequila, cycles] = both_engines("wood", |scene| {
            let wood = Procedural::Wood {
                axis: Axis::Z,
                frequency: 2.3,
                distortion: 0.002,
                grain: 0.0,
                fractal: tequila_texture::noise::Fractal::default(),
            };
            let ramp = Gradient::new(vec![
                Stop { at: 0.0, color: Spectrum { r: 0.02, g: 0.01, b: 0.005 } },
                Stop { at: 0.5, color: Spectrum { r: 0.3, g: 0.1, b: 0.02 } },
                Stop { at: 0.5, color: Spectrum { r: 0.9, g: 0.6, b: 0.3 } },
                Stop { at: 1.0, color: Spectrum { r: 0.6, g: 0.3, b: 0.1 } },
            ]);
            procedural_material(scene, wood, ramp);
        });
        let (a, b) = (inside_the_sphere(&tequila), inside_the_sphere(&cycles));
        let ((ma, sa), (mb, _)) = (mean_and_spread(&a), mean_and_spread(&b));
        assert!(sa > 60.0, "Tequila's wood has no rings: spread {sa:.1}");
        let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(&b) {
            cov += (x - ma) * (y - mb);
            va += (x - ma) * (x - ma);
            vb += (y - mb) * (y - mb);
        }
        let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
        assert!(correlation > 0.85, "the rings are not where Tequila's renderer puts them: correlation {correlation:.2}");
        // And the ramp's browns, channel by channel.
        let (fa, fb) = (channel_means(&tequila), channel_means(&cycles));
        for c in 0..3 {
            assert!((fb[c] / fa[c] - 1.0).abs() < 0.1, "Tequila's wood is {fa:.1?}, Cycles' {fb:.1?}");
        }
    }

    /// Procedural noise comes out of Cycles as Tequila's renderer makes it:
    /// not the same blotches - the two hash their lattices differently - but
    /// as bright on average and as contrasty, octave for octave, read
    /// straight through a black-to-white ramp. Cycles read it as a flat grey.
    /// Skipped where Cycles is not installed.
    #[test]
    fn procedural_noise_has_tequila_contrast_in_cycles() {
        use tequila_core::spectrum::Spectrum;
        use tequila_texture::gradient::Gradient;
        use tequila_texture::noise::Fractal;
        use tequila_texture::procedural::Procedural;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        for (name, procedural) in [
            ("noise", Procedural::Noise { fractal: Fractal { octaves: 3, ..Fractal::default() }, frequency: 5.0 }),
            ("marble", Procedural::Marble { axis: tequila_texture::projection::Axis::Y, frequency: 6.0, distortion: 4.0, fractal: Fractal::default() }),
        ] {
            let [tequila, cycles] = both_engines(name, |scene| {
                procedural_material(scene, procedural.clone(), Gradient::two(Spectrum::splat(0.02), Spectrum::splat(0.9)));
            });
            let ((ma, sa), (mb, sb)) = (mean_and_spread(&inside_the_sphere(&tequila)), mean_and_spread(&inside_the_sphere(&cycles)));
            assert!(sa > 25.0, "Tequila's {name} is flat: spread {sa:.1}");
            assert!((mb / ma - 1.0).abs() < 0.12, "Tequila's {name} is {ma:.1} bright, Cycles' {mb:.1}");
            assert!((sb / sa - 1.0).abs() < 0.25, "Tequila's {name} spreads {sa:.1}, Cycles' {sb:.1}");
        }
    }

    /// A translucent part comes out of Cycles as out of Tequila's renderer:
    /// as bright, near-black or grey, a short radius or a long one. Tequila
    /// read the radius as a diffusion profile's width where Cycles reads it
    /// as four pi mean free paths, so its media were twenty to fifty times
    /// thinner: a near-black stone with a five-centimetre radius came out at
    /// 420 to Cycles' 32, brighter than a grey one, seen through. Skipped
    /// where Cycles is not installed.
    #[test]
    fn subsurface_renders_alike_in_cycles() {
        use tequila_core::spectrum::Spectrum;
        use tequila_materials::material_type::MaterialType;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let mut wrong = Vec::new();
        for (grey, radius) in [(0.027f32, 0.05f32), (0.027, 0.002), (0.5, 0.05), (0.5, 0.002)] {
            let [tequila, cycles] = both_engines_scaled(&format!("subsurface-{grey}-{radius}"), 0.025, |scene| {
                let mut translucent = MaterialType::Translucent.defaults();
                translucent.base_color = Spectrum::splat(grey);
                translucent.subsurface_color = Spectrum::splat(grey);
                translucent.subsurface_weight = 1.0;
                translucent.subsurface_radius = Spectrum::splat(radius);
                translucent.specular_roughness = 0.05;
                scene.environment = Arc::new(tequila_scene::demo::preview_environment());
                scene.materials[0] = translucent;
            });
            let ((ma, _), (mb, _)) = (mean_and_spread(&inside_the_sphere(&tequila)), mean_and_spread(&inside_the_sphere(&cycles)));
            eprintln!("grey {grey}, radius {radius}: Tequila {ma:.1}, Cycles {mb:.1}");
            if (mb / ma.max(1.0) - 1.0).abs() > 0.15 {
                wrong.push(format!("grey {grey}, radius {radius}: Tequila {ma:.1}, Cycles {mb:.1}"));
            }
        }
        assert!(wrong.is_empty(), "subsurface renders differently in Cycles:
{}", wrong.join("
"));
    }

    /// An anisotropic highlight turns as far in Cycles as in Tequila's
    /// renderer. Tequila's rotation runs 0 to 1 over a half turn - an
    /// anisotropic lobe turned half way round is the same lobe - and Cycles'
    /// over a whole one, so the rotation went over unscaled turned every
    /// highlight twice as far: a quarter turn became a half, which is none,
    /// and a carbon weave's warp and weft caught the light alike. Skipped
    /// where Cycles is not installed.
    #[test]
    fn anisotropy_turns_alike_in_cycles() {
        use tequila_materials::material_type::MaterialType;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let mut wrong = Vec::new();
        for rotation in [0.0f32, 0.25, 0.5] {
            let [tequila, cycles] = both_engines_scaled(&format!("anisotropy-{rotation}"), 0.025, |scene| {
                let mut m = MaterialType::Anisotropic.defaults();
                m.specular_roughness = 0.35;
                m.specular_roughness_anisotropy = 0.95;
                m.specular_rotation = rotation;
                scene.environment = Arc::new(tequila_scene::demo::preview_environment());
                scene.materials[0] = m;
            });
            let (a, b) = (inside_the_sphere(&tequila), inside_the_sphere(&cycles));
            let ((ma, _), (mb, _)) = (mean_and_spread(&a), mean_and_spread(&b));
            let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
            for (x, y) in a.iter().zip(&b) {
                cov += (x - ma) * (y - mb);
                va += (x - ma) * (x - ma);
                vb += (y - mb) * (y - mb);
            }
            let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
            eprintln!("rotation {rotation}: correlation {correlation:.3}, Tequila {ma:.1}, Cycles {mb:.1}");
            if correlation < 0.96 {
                wrong.push(format!("rotation {rotation}: correlation {correlation:.3}"));
            }
        }
        assert!(wrong.is_empty(), "anisotropy turns differently in Cycles: {}", wrong.join(", "));
    }

    /// An anisotropic part with no UVs - a CAD body, as most imports are -
    /// comes out of Cycles as out of Tequila's renderer, its lobe in the
    /// frame the normal alone gives. Cycles' UV-map tangent is zero on a mesh
    /// without UVs, and Principled made a NaN of it: every carbon, kevlar or
    /// brushed part on a STEP model rendered black. Skipped where Cycles is
    /// not installed.
    #[test]
    fn anisotropy_without_uvs_renders_in_cycles() {
        use tequila_materials::material_type::MaterialType;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let mut wrong = Vec::new();
        for rotation in [0.0f32, 0.25] {
            let [tequila, cycles] = both_engines_scaled(&format!("anisotropy-no-uvs-{rotation}"), 0.025, |scene| {
                for mesh in &mut scene.meshes {
                    let mut bare = (**mesh).clone();
                    bare.uvs.clear();
                    bare.tangents.clear();
                    *mesh = Arc::new(bare);
                }
                let mut m = MaterialType::Anisotropic.defaults();
                m.specular_roughness = 0.35;
                m.specular_roughness_anisotropy = 0.95;
                m.specular_rotation = rotation;
                scene.environment = Arc::new(tequila_scene::demo::preview_environment());
                scene.materials[0] = m;
            });
            let (a, b) = (inside_the_sphere(&tequila), inside_the_sphere(&cycles));
            let ((ma, _), (mb, _)) = (mean_and_spread(&a), mean_and_spread(&b));
            let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
            for (x, y) in a.iter().zip(&b) {
                cov += (x - ma) * (y - mb);
                va += (x - ma) * (x - ma);
                vb += (y - mb) * (y - mb);
            }
            let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
            eprintln!("rotation {rotation}: correlation {correlation:.3}, Tequila {ma:.1}, Cycles {mb:.1}");
            if (mb / ma.max(1.0) - 1.0).abs() > 0.15 || correlation < 0.96 {
                wrong.push(format!("rotation {rotation}: correlation {correlation:.3}, Tequila {ma:.1}, Cycles {mb:.1}"));
            }
        }
        assert!(wrong.is_empty(), "anisotropy without UVs renders differently in Cycles: {}", wrong.join(", "));
    }

    /// The direction a material's graph draws for its anisotropy
    /// (`Param::Tangent`) points the highlight in Cycles as in Tequila's
    /// renderer, on a part without UVs: a constant one up the part, and one
    /// that varies - circles about the part's axis, as a spun finish is
    /// turned. Cycles took no direction from the graph, so a brushed
    /// pattern's lines and its highlight could run at right angles. Skipped
    /// where Cycles is not installed.
    #[test]
    fn a_drawn_tangent_renders_alike_in_cycles() {
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, Value};
        use tequila_materials::material_type::{MaterialType, Param};
        use tequila_materials::nodes::{Setting, ShaderNode};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let mut wrong = Vec::new();
        for drawn in ["up", "around"] {
            let [tequila, cycles] = both_engines_scaled(&format!("drawn-tangent-{drawn}"), 0.025, |scene| {
                for mesh in &mut scene.meshes {
                    let mut bare = (**mesh).clone();
                    bare.uvs.clear();
                    bare.tangents.clear();
                    *mesh = Arc::new(bare);
                }
                let mut m = MaterialType::Anisotropic.defaults();
                m.specular_roughness = 0.35;
                m.specular_roughness_anisotropy = 0.95;
                let mut graph = MaterialGraph::from_material(m);
                let output = graph.output();
                if drawn == "up" {
                    let constant = graph.add_node(Node::new(NodeKind::Constant(Value::Vector(Vec3::Y))));
                    graph.connect(constant, output, Socket::Param(Param::Tangent));
                } else {
                    let coordinates = ShaderNode::from_key("texture_coordinate").expect("a node");
                    let object = coordinates.outputs().iter().position(|o| o.name == "Object").expect("an Object output") as u8;
                    let coordinates = graph.add_node(Node::new(NodeKind::Node(coordinates)));
                    let mut cross = ShaderNode::from_key("vector_math").expect("a node");
                    assert!(cross.set("operation", Setting::Choice("cross_product")));
                    let mut cross = Node::new(NodeKind::Node(cross));
                    cross.literals.insert(Socket::Input(1), Value::Vector(Vec3::Y));
                    let cross = graph.add_node(cross);
                    graph.connect_output(coordinates, object, cross, Socket::Input(0));
                    graph.connect(cross, output, Socket::Param(Param::Tangent));
                }
                let program = graph.compile().expect("compiles");
                scene.environment = Arc::new(tequila_scene::demo::preview_environment());
                scene.materials[0] = graph.material().clone();
                *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                    .with_programs(vec![Some(Arc::new(program))], Arc::new(tequila_render_core::shade::TextureTable::new()));
            });
            let (a, b) = (inside_the_sphere(&tequila), inside_the_sphere(&cycles));
            let ((ma, _), (mb, _)) = (mean_and_spread(&a), mean_and_spread(&b));
            let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
            for (x, y) in a.iter().zip(&b) {
                cov += (x - ma) * (y - mb);
                va += (x - ma) * (x - ma);
                vb += (y - mb) * (y - mb);
            }
            let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
            eprintln!("{drawn}: correlation {correlation:.3}, Tequila {ma:.1}, Cycles {mb:.1}");
            if (mb / ma.max(1.0) - 1.0).abs() > 0.15 || correlation < 0.96 {
                wrong.push(format!("{drawn}: correlation {correlation:.3}, Tequila {ma:.1}, Cycles {mb:.1}"));
            }
        }
        assert!(wrong.is_empty(), "a drawn tangent renders differently in Cycles: {}", wrong.join(", "));
    }

    /// A velvet's sheen comes out of Cycles as out of Tequila's renderer,
    /// tight pile or loose: a crimson fuzz over a dark base, lit from one
    /// side, lands on the same pixels at the same level - and a black fuzz
    /// leaves the base alone in both. Tequila's sheen was Conty-Kulla's
    /// charlie lobe, which hardly brightens as the pile roughens where
    /// Cycles' fibre layer does a great deal (0.26 correlated at roughness
    /// 0.3, Tequila 124 to Cycles 153 at 0.9), and it dimmed the base
    /// under a black fuzz by a fifth. Skipped where Cycles is not installed.
    #[test]
    fn velvet_sheen_renders_alike_in_cycles() {
        use tequila_core::spectrum::{spectrum, Spectrum};
        use tequila_materials::material_type::MaterialType;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let mut wrong = Vec::new();
        for (roughness, fuzz) in [(0.3f32, spectrum(0.75, 0.09, 0.14)), (0.6, spectrum(0.75, 0.09, 0.14)), (0.9, spectrum(0.75, 0.09, 0.14)), (0.9, Spectrum::ZERO)] {
            let [tequila, cycles] = both_engines_scaled(&format!("velvet-{roughness}-{}", fuzz.max_component()), 0.025, |scene| {
                let mut velvet = MaterialType::Velvet.defaults();
                velvet.base_color = spectrum(0.11, 0.003, 0.008);
                velvet.fuzz_color = fuzz;
                velvet.fuzz_roughness = roughness;
                scene.environment = Arc::new(tequila_scene::demo::preview_environment());
                scene.materials[0] = velvet;
            });
            let (a, b) = (inside_the_sphere(&tequila), inside_the_sphere(&cycles));
            let ((ma, _), (mb, _)) = (mean_and_spread(&a), mean_and_spread(&b));
            let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
            for (x, y) in a.iter().zip(&b) {
                cov += (x - ma) * (y - mb);
                va += (x - ma) * (x - ma);
                vb += (y - mb) * (y - mb);
            }
            let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
            eprintln!("roughness {roughness}, fuzz {fuzz:?}: correlation {correlation:.3}, Tequila {ma:.1}, Cycles {mb:.1}");
            if correlation < 0.93 || (mb / ma.max(1.0) - 1.0).abs() > 0.08 {
                wrong.push(format!("roughness {roughness}, fuzz {fuzz:?}: correlation {correlation:.3}, Tequila {ma:.1}, Cycles {mb:.1}"));
            }
        }
        assert!(wrong.is_empty(), "velvet renders differently in Cycles:
{}", wrong.join("
"));
    }

    /// The standard library's presets come out of Cycles as out of
    /// Tequila's renderer: their patterns on the same pixels, and the
    /// bumps their node graphs raise tilting the surface the same way.
    /// Skipped where Cycles is not installed.
    #[test]
    fn library_presets_render_alike_in_cycles() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let folder = crate::library_tab::standard_library_path();
        let (entries, _) = tequila_format::library::read_directory(&folder);
        let mut wrong = Vec::new();
        // Each at a size its pattern is resolved at, 96 pixels across: the
        // knurl's grooves and the hex grip's hexagons are a millimetre or two
        // apart, finer than a pixel on a
        // five-centimetre ball, and a bump finer than a pixel is estimated
        // differently by every renderer.
        for (name, scale) in [
            ("Copper Hammered", 0.025),
            ("Steel Knurled", 0.006),
            ("Marble White", 0.025),
            ("Marble Black", 0.025),
            ("Travertine", 0.025),
            ("Granite Rosa", 0.025),
            ("Concrete", 0.025),
            ("Stoneware Speckled", 0.025),
            ("Carbon Fibre", 0.025),
            ("Wood White Oak", 0.025),
            ("Wood Olive", 0.025),
            ("Cork", 0.025),
            ("Denim Indigo", 0.025),
            ("Velvet Crimson", 0.025),
            ("Terrazzo", 0.025),
            ("Leather Tan", 0.025),
            ("Steel Rusted", 0.025),
            ("Powder Coat Signal Red", 0.025),
            // The plastics and metals of 2026-10-12: stamped, cut through,
            // brushed and engraved along a direction the graph draws,
            // pressed from chips, forged in layers, heat coloured.
            ("Aluminium Tread Plate", 0.025),
            ("Steel Perforated Round", 0.025),
            ("Stainless Brushed", 0.025),
            ("Steel Geneva Stripes", 0.025),
            ("Plastic Hex Grip", 0.006),
            ("Recycled Plastic Confetti", 0.025),
            ("Damascus Steel", 0.025),
            ("Titanium Heat Tinted", 0.025),
        ] {
            let entry = entries.iter().find(|e| e.name == name).unwrap_or_else(|| panic!("no preset {name}"));
            let graph = entry.graph.clone();
            let [tequila, cycles] = both_engines_scaled(&format!("preset-{name}"), scale, |scene| {
                let program = graph.compile().expect("compiles");
                scene.materials[0] = graph.material().clone();
                // Lit from one side, as the library's own studio does: a bump
                // under even light is hardly there to compare.
                scene.environment = Arc::new(tequila_scene::demo::preview_environment());
                *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                    .with_programs(vec![Some(Arc::new(program))], Arc::new(tequila_render_core::shade::TextureTable::new()));
            });
            let (a, b) = (inside_the_sphere(&tequila), inside_the_sphere(&cycles));
            let ((ma, sa), (mb, sb)) = (mean_and_spread(&a), mean_and_spread(&b));
            let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
            for (x, y) in a.iter().zip(&b) {
                cov += (x - ma) * (y - mb);
                va += (x - ma) * (x - ma);
                vb += (y - mb) * (y - mb);
            }
            let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
            eprintln!("{name}: correlation {correlation:.3}, Tequila {ma:.1} spread {sa:.1}, Cycles {mb:.1} spread {sb:.1}");
            if correlation < 0.85 || (mb / ma.max(1.0) - 1.0).abs() > 0.15 {
                wrong.push(format!("{name}: correlation {correlation:.3}, Tequila {ma:.1}, Cycles {mb:.1}"));
            }
        }
        assert!(wrong.is_empty(), "these presets render differently in Cycles:
{}", wrong.join("
"));
    }

    /// A board 121 x 25 x 110 mm in `material`, under a panel light, seen
    /// close up over one corner, near enough for a pattern's grain to span
    /// pixels: every length `numbers` times its size in
    /// metres, as a model that came in at the wrong unit has them, and the
    /// light and the camera set up on it at that size.
    fn board(material: &tequila_scene::MaterialDefinition, numbers: f32) -> tequila_scene::SceneDocument {
        use tequila_core::math::Transform;
        use tequila_scene::command::{reserve_keys, ImportPayload};
        use tequila_scene::lighting::{Intensity, LightSettings, LightShape, SavedCamera};
        use tequila_scene::{Command, CommandBus, Mesh, NodeKind, SceneDocument, SceneNode};
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let (meshes, materials, ids) = reserve_keys(&document, 1, 1, 1);
        // A closed box, four corners to a face so each face is flat.
        let half = vec3(0.0605, 0.0125, 0.055) * numbers;
        let centre = vec3(0.0, half.y, 0.0);
        let (mut positions, mut normals, mut indices) = (Vec::new(), Vec::new(), Vec::new());
        for (n, u, v) in [
            (Vec3::X, Vec3::Y, Vec3::Z),
            (-Vec3::X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::Z, Vec3::X),
            (-Vec3::Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (-Vec3::Z, Vec3::Y, Vec3::X),
        ] {
            let first = positions.len() as u32;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                positions.push(centre + (n + u * a + v * b) * half);
                normals.push(n);
            }
            indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
        }
        let mut node = SceneNode::new(ids[0], "Hangboard", NodeKind::Part { mesh: meshes[0] });
        node.material = Some(materials[0]);
        let panel = LightSettings {
            shape: LightShape::Area { width: 0.1 * numbers, height: 0.1 * numbers, disc: false, two_sided: false },
            intensity: Intensity::Lumens(0.5 * numbers * numbers),
            ..Default::default()
        };
        let eye = vec3(0.064, 0.084, 0.081) * numbers;
        let target = vec3(0.03, 0.025, 0.03) * numbers;
        for command in [
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(meshes[0], Mesh::new(positions, indices).with_normals(normals))],
                    textures: Vec::new(),
                    materials: vec![(materials[0], material.clone())],
                    nodes: vec![node],
                    roots: vec![ids[0]],
                }),
            },
            Command::AddLight { parent: None, name: "Panel".into(), settings: Box::new(panel) },
            Command::SetCamera {
                index: 0,
                camera: Box::new(SavedCamera { name: "Camera 1".into(), camera: Camera::look_at(eye, target, Vec3::Y), target }),
            },
        ] {
            bus.apply(&mut document, command).unwrap();
        }
        // Over the board, facing down: an area light emits along its local
        // +Z, the cross of its width and its height.
        let light = document.tree.iter().find(|n| n.kind.light().is_some()).unwrap().id;
        let place = Transform::translation(vec3(-0.05, 0.3, 0.1) * numbers)
            * Transform::rotation(Vec3::X, std::f32::consts::FRAC_PI_2);
        bus.apply(&mut document, Command::set_transform(light, place)).unwrap();
        document
    }

    /// `document` as a build from before 2026-10-13 saved it, with its
    /// numbers in units of `metres_per_unit`, opened by this one.
    fn opened_from_an_old_file(document: &tequila_scene::SceneDocument, metres_per_unit: f64) -> tequila_scene::SceneDocument {
        let bytes = tequila_format::write(document, &tequila_format::Preserved::default());
        let archive = tequila_format::zip::ZipArchive::read(&bytes).unwrap();
        let mut old = tequila_format::zip::ZipWriter::new();
        for name in archive.names() {
            let mut member = archive.get(name).unwrap().to_vec();
            if name == "scene.json" {
                let text = String::from_utf8(member).unwrap();
                let key = text.find("\"metres_per_unit\"").unwrap();
                let colon = key + text[key..].find(':').unwrap() + 1;
                let end = colon + text[colon..].find(|c: char| c == ',' || c == '}').unwrap();
                member = format!("{}{metres_per_unit}{}", &text[..colon], &text[end..]).into_bytes();
            }
            old.add(name.clone(), member);
        }
        tequila_format::read(&old.finish()).unwrap().document
    }

    /// Cork on the board, reached every way a scene's units can be set,
    /// rendered by `engine`, against the board built in metres. Returns how
    /// far each is from it, and how far the control is: the same board left
    /// a thousand times its size, which is what every one of these used to
    /// render.
    fn cork_by_every_unit_route(engine: tequila_scene::render::RenderEngine) -> (Vec<(&'static str, f64)>, f64) {
        use tequila_core::units::LengthUnit;
        use tequila_scene::{Command, CommandBus, ConversionMode};
        let folder = crate::library_tab::standard_library_path();
        let (entries, _) = tequila_format::library::read_directory(&folder);
        let cork = entries.iter().find(|e| e.name == "Cork").expect("the Cork preset").definition();
        let set_units = |mut document: tequila_scene::SceneDocument, mode| {
            CommandBus::new().apply(&mut document, Command::SetUnits { unit: LengthUnit::Millimetre, mode }).unwrap();
            document
        };

        let reference = board(&cork, 1.0);
        let routes = [
            // Imported with its unit stated, then shown in millimetres.
            ("shown in mm", set_units(reference.clone(), ConversionMode::KeepSceneSize)),
            // Came in at its raw numbers, 121 m across, and was told its
            // numbers are millimetres.
            ("numbers read as mm", set_units(board(&cork, 1000.0), ConversionMode::ScaleScene)),
            // Saved by an old build with its numbers in millimetres.
            ("old mm file", opened_from_an_old_file(&set_units(board(&cork, 1000.0), ConversionMode::KeepSceneSize), 0.001)),
        ];
        let control = board(&cork, 1000.0);

        let render = |name: &str, document: &tequila_scene::SceneDocument| -> Vec<f64> {
            let environment = Arc::new(tequila_scene::demo::preview_environment());
            let (scene, _) = tequila_scene::snapshot::snapshot(document, environment, &mut tequila_scene::snapshot::MeshCache::new());
            let directory = scratch(&format!("tequila-units-{}-{}", engine.key(), name.replace(' ', "-")));
            let renderer = BatchRenderer::new();
            // Few samples: the routes render the same scene, so they agree
            // sample for sample, and the control is off by the whole pattern.
            // At 64 a picture of cork took five seconds here, and on GitHub's
            // four-core Windows runner, among the other tests, passed the
            // two minutes the batch renderer is given.
            let mut job = job(1, &directory, RenderSettings { engine, width: 128, height: 96, max_samples: 16, denoise: false, ..Default::default() });
            job.scene = Arc::new(scene);
            job.camera = document.cameras.active().camera;
            job.config = IntegratorConfig::product();
            renderer.submit(job);
            match wait_for_finish(&renderer) {
                BatchEvent::Finished { preview, .. } => blocks(&preview, 128, 96),
                BatchEvent::Failed { message, .. } => panic!("{name}: {message}"),
                _ => unreachable!(),
            }
        };
        let expected = render("reference", &reference);
        let mean = expected.iter().sum::<f64>() / expected.len() as f64;
        assert!(mean > 20.0, "the reference came out black: {mean:.1}");
        let distance = |image: &[f64]| expected.iter().zip(image).map(|(a, b)| (a - b).abs()).sum::<f64>() / expected.len() as f64;
        let far = routes.iter().map(|(name, document)| (*name, distance(&render(name, document)))).collect();
        (far, distance(&render("control", &control)))
    }

    /// An image as the mean grey of its 2 x 2 pixel blocks: the grain a
    /// person sees, with some of a 64-sample render's noise averaged away.
    fn blocks(rgba: &[u8], width: usize, height: usize) -> Vec<f64> {
        let mut out = Vec::new();
        for by in 0..height / 2 {
            for bx in 0..width / 2 {
                let mut sum = 0.0;
                for y in by * 2..by * 2 + 2 {
                    for x in bx * 2..bx * 2 + 2 {
                        let p = &rgba[(y * width + x) * 4..][..3];
                        sum += f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]);
                    }
                }
                out.push(sum / 12.0);
            }
        }
        out
    }

    fn assert_cork_renders_alike(engine: tequila_scene::render::RenderEngine) {
        let (routes, control) = cork_by_every_unit_route(engine);
        eprintln!("{}: {routes:?}, control {control:.2}", engine.key());
        for (route, distance) in routes {
            assert!(
                distance < 2.0 && distance < control / 4.0,
                "{}: cork {route} is {distance:.2} grey levels from the board built in metres; \
                 the board left a thousand times its size is {control:.2}",
                engine.key()
            );
        }
    }

    /// Cork on a 121 x 110 x 25 mm part renders the same however the
    /// scene's units were set: shown in millimetres, read as millimetres
    /// after coming in a thousand times too big, or opened from a file that
    /// kept its numbers in millimetres. Each of those used to render the
    /// pattern a thousandth of its size, and the control shows that is far
    /// from the same picture.
    #[test]
    fn cork_renders_alike_however_the_units_were_set() {
        assert_cork_renders_alike(tequila_scene::render::RenderEngine::Tequila);
    }

    /// The same through Cycles. Skipped where Cycles is not installed.
    #[test]
    fn cork_renders_alike_in_cycles_however_the_units_were_set() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        assert_cork_renders_alike(tequila_scene::render::RenderEngine::CyclesCpu);
    }

    /// Blender's nodes shade the same in Cycles as in Tequila's renderer,
    /// value for value: each node's output glowing unlit on the sphere
    /// lands on the same pixels at the same levels in both. They share
    /// Cycles' algorithms, so anything less is a translation that is
    /// wrong. Skipped where Cycles is not installed.
    #[test]
    fn blender_nodes_shade_alike_in_both_engines() {
        use tequila_materials::graph::{MaterialGraph, Node, NodeId, NodeKind, Socket, Value};
        use tequila_materials::material_type::MaterialType;
        use tequila_materials::nodes::{Setting, ShaderNode};
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        // A node with these settings and literals.
        fn add(graph: &mut MaterialGraph, key: &str, settings: &[(&str, Setting)], literals: &[(u8, Value)]) -> NodeId {
            let mut node = ShaderNode::from_key(key).unwrap_or_else(|| panic!("no node {key}"));
            for (k, v) in settings {
                assert!(node.set(k, *v), "{key} has no setting {k} = {v:?}");
            }
            let mut built = Node::new(NodeKind::Node(node));
            for (socket, value) in literals {
                built.literals.insert(Socket::Input(*socket), *value);
            }
            graph.add_node(built)
        }
        fn f(v: f32) -> Value {
            Value::Float(v)
        }
        fn v(x: f32, y: f32, z: f32) -> Value {
            Value::Vector(vec3(x, y, z))
        }
        use Setting::{Bool, Choice, Int};
        type Build = Box<dyn Fn(&mut MaterialGraph) -> (NodeId, u8)>;
        // A wave read through a three-colour ramp going `interpolation`'s way.
        fn ramp(g: &mut MaterialGraph, interpolation: tequila_texture::gradient::Interpolation) -> (NodeId, u8) {
            use tequila_core::spectrum::Spectrum;
            use tequila_texture::gradient::{Gradient, Stop};
            let wave = add(g, "wave_texture", &[], &[(1, f(2.0))]);
            let stops = vec![
                Stop { at: 0.1, color: Spectrum { r: 0.8, g: 0.1, b: 0.05 } },
                Stop { at: 0.5, color: Spectrum { r: 0.1, g: 0.6, b: 0.2 } },
                Stop { at: 0.85, color: Spectrum { r: 0.1, g: 0.2, b: 0.9 } },
            ];
            let ramp = g.add_node(Node::new(NodeKind::Gradient { gradient: Box::new(Gradient::new(stops).with_interpolation(interpolation)) }));
            g.connect_output(wave, 1, ramp, Socket::Input(0));
            (ramp, 0)
        }
        let absolute = |g: &mut MaterialGraph, from: NodeId, output: u8| {
            let abs = add(g, "vector_math", &[("operation", Choice("absolute"))], &[]);
            g.connect_output(from, output, abs, Socket::Input(0));
            (abs, 0)
        };
        let cases: Vec<(&str, Build)> = vec![
            ("noise", Box::new(|g| (add(g, "noise_texture", &[], &[(2, f(3.0)), (3, f(3.0))]), 0))),
            ("noise-4d-colour", Box::new(|g| (add(g, "noise_texture", &[("dimensions", Choice("4D"))], &[(1, f(0.3)), (2, f(2.0)), (8, f(0.5))]), 1))),
            ("noise-ridged", Box::new(|g| (add(g, "noise_texture", &[("type", Choice("ridged_multifractal"))], &[(2, f(2.0)), (3, f(4.0)), (6, f(1.0)), (7, f(2.0))]), 0))),
            ("noise-hetero-2d", Box::new(|g| (add(g, "noise_texture", &[("type", Choice("hetero_terrain")), ("dimensions", Choice("2D"))], &[(2, f(2.0)), (6, f(0.5))]), 0))),
            ("voronoi-colour", Box::new(|g| (add(g, "voronoi_texture", &[], &[(2, f(4.0))]), 1))),
            ("voronoi-edge", Box::new(|g| (add(g, "voronoi_texture", &[("feature", Choice("distance_to_edge"))], &[(2, f(4.0))]), 0))),
            ("voronoi-smooth", Box::new(|g| (add(g, "voronoi_texture", &[("feature", Choice("smooth_f1")), ("dimensions", Choice("2D"))], &[(2, f(4.0)), (6, f(0.6))]), 0))),
            ("voronoi-f2-position", Box::new(|g| (add(g, "voronoi_texture", &[("feature", Choice("f2")), ("metric", Choice("chebychev"))], &[(2, f(3.0))]), 2))),
            ("voronoi-fractal", Box::new(|g| (add(g, "voronoi_texture", &[("normalize", Bool(true))], &[(2, f(3.0)), (3, f(2.0))]), 0))),
            ("wave-rings", Box::new(|g| (add(g, "wave_texture", &[("type", Choice("rings")), ("direction", Choice("diagonal")), ("profile", Choice("saw"))], &[(1, f(2.0)), (2, f(2.0)), (3, f(2.0))]), 1))),
            ("wave-bands", Box::new(|g| (add(g, "wave_texture", &[("direction", Choice("diagonal"))], &[(1, f(3.0))]), 0))),
            ("magic", Box::new(|g| (add(g, "magic_texture", &[("depth", Int(3))], &[(1, f(2.0))]), 0))),
            ("brick", Box::new(|g| (add(g, "brick_texture", &[], &[(4, f(2.0))]), 0))),
            ("checker", Box::new(|g| (add(g, "checker_texture", &[], &[(3, f(3.0))]), 0))),
            ("gradient-generated", Box::new(|g| {
                let coords = add(g, "texture_coordinate", &[], &[]);
                let ramp = add(g, "gradient_texture", &[("type", Choice("quadratic"))], &[]);
                g.connect_output(coords, 0, ramp, Socket::Input(0));
                (ramp, 1)
            })),
            ("white-noise-cells", Box::new(|g| {
                let coords = add(g, "texture_coordinate", &[], &[]);
                let scaled = add(g, "vector_math", &[("operation", Choice("scale"))], &[(3, f(4.0))]);
                g.connect_output(coords, 3, scaled, Socket::Input(0));
                let cells = add(g, "vector_math", &[("operation", Choice("floor"))], &[]);
                g.connect_output(scaled, 0, cells, Socket::Input(0));
                let noise = add(g, "white_noise_texture", &[], &[]);
                g.connect_output(cells, 0, noise, Socket::Input(0));
                (noise, 1)
            })),
            ("gabor", Box::new(|g| (add(g, "gabor_texture", &[], &[(1, f(4.0)), (3, f(0.6))]), 0))),
            ("gabor-3d-phase", Box::new(|g| (add(g, "gabor_texture", &[("type", Choice("3D"))], &[(1, f(3.0)), (2, f(1.5))]), 1))),
            ("gabor-intensity", Box::new(|g| (add(g, "gabor_texture", &[], &[(1, f(4.0)), (4, f(1.2))]), 2))),
            ("layer-weight-facing", Box::new(|g| (add(g, "layer_weight", &[], &[(0, f(0.3))]), 1))),
            ("layer-weight-fresnel", Box::new(|g| (add(g, "layer_weight", &[], &[(0, f(0.3))]), 0))),
            ("fresnel", Box::new(|g| (add(g, "fresnel", &[], &[(0, f(1.45))]), 0))),
            ("geometry-normal", Box::new(move |g| {
                let geometry = add(g, "geometry", &[], &[]);
                absolute(g, geometry, 1)
            })),
            ("geometry-position", Box::new(|g| {
                let geometry = add(g, "geometry", &[], &[]);
                let half = add(g, "vector_math", &[("operation", Choice("scale"))], &[(3, f(0.5))]);
                g.connect_output(geometry, 0, half, Socket::Input(0));
                (half, 0)
            })),
            ("geometry-incoming", Box::new(move |g| {
                let geometry = add(g, "geometry", &[], &[]);
                absolute(g, geometry, 3)
            })),
            ("reflection", Box::new(move |g| {
                let coords = add(g, "texture_coordinate", &[], &[]);
                absolute(g, coords, 4)
            })),
            ("object", Box::new(|g| {
                let coords = add(g, "texture_coordinate", &[], &[]);
                let wrapped = add(g, "vector_math", &[("operation", Choice("fraction"))], &[]);
                g.connect_output(coords, 3, wrapped, Socket::Input(0));
                (wrapped, 0)
            })),
            ("object-normal", Box::new(move |g| {
                let coords = add(g, "texture_coordinate", &[], &[]);
                absolute(g, coords, 1)
            })),
            ("uv", Box::new(|g| {
                let coords = add(g, "texture_coordinate", &[], &[]);
                (coords, 2)
            })),
            ("object-random", Box::new(|g| (add(g, "object_info", &[], &[]), 1))),
            // Seen directly: a camera ray, and no reflection.
            ("light-path-camera", Box::new(|g| (add(g, "light_path", &[], &[]), 0))),
            ("light-path-glossy", Box::new(|g| (add(g, "light_path", &[], &[]), 3))),
            ("object-location", Box::new(move |g| {
                let info = add(g, "object_info", &[], &[]);
                absolute(g, info, 0)
            })),
            ("mix-overlay", Box::new(|g| {
                let a = add(g, "noise_texture", &[], &[(2, f(3.0))]);
                let b = add(g, "voronoi_texture", &[], &[(2, f(3.0))]);
                let mix = add(g, "mix", &[("blend_type", Choice("overlay"))], &[(0, f(0.7))]);
                g.connect_output(a, 1, mix, Socket::Input(1));
                g.connect_output(b, 1, mix, Socket::Input(2));
                (mix, 0)
            })),
            ("math-chain", Box::new(|g| {
                let noise = add(g, "noise_texture", &[], &[(2, f(2.0))]);
                let times = add(g, "math", &[("operation", Choice("multiply"))], &[(1, f(20.0))]);
                g.connect_output(noise, 0, times, Socket::Input(0));
                let sine = add(g, "math", &[("operation", Choice("sine"))], &[]);
                g.connect_output(times, 0, sine, Socket::Input(0));
                let fit = add(g, "map_range", &[("interpolation", Choice("smoothstep"))], &[(1, f(-1.0)), (2, f(1.0))]);
                g.connect_output(sine, 0, fit, Socket::Input(0));
                (fit, 0)
            })),
            ("hsv", Box::new(|g| {
                let checker = add(g, "checker_texture", &[], &[(3, f(3.0)), (1, Value::Color(tequila_core::spectrum::Spectrum { r: 0.8, g: 0.3, b: 0.1 }))]);
                let hsv = add(g, "hue_saturation", &[], &[(0, f(0.8)), (1, f(1.4))]);
                g.connect_output(checker, 0, hsv, Socket::Input(4));
                (hsv, 0)
            })),
            ("contrast-gamma-invert", Box::new(|g| {
                let magic = add(g, "magic_texture", &[], &[(1, f(2.0))]);
                let bc = add(g, "brightness_contrast", &[], &[(1, f(0.1)), (2, f(0.4))]);
                g.connect_output(magic, 0, bc, Socket::Input(0));
                let gamma = add(g, "gamma", &[], &[(1, f(2.2))]);
                g.connect_output(bc, 0, gamma, Socket::Input(0));
                let invert = add(g, "invert", &[], &[(0, f(0.5))]);
                g.connect_output(gamma, 0, invert, Socket::Input(1));
                (invert, 0)
            })),
            ("separate-combine-hsv", Box::new(|g| {
                let magic = add(g, "magic_texture", &[], &[(1, f(2.0))]);
                let split = add(g, "separate_color", &[("mode", Choice("hsv"))], &[]);
                g.connect_output(magic, 0, split, Socket::Input(0));
                let join = add(g, "combine_color", &[], &[]);
                g.connect_output(split, 2, join, Socket::Input(0));
                g.connect_output(split, 0, join, Socket::Input(1));
                g.connect_output(split, 1, join, Socket::Input(2));
                (join, 0)
            })),
            ("mapping", Box::new(|g| {
                let coords = add(g, "texture_coordinate", &[], &[]);
                let mapping = add(g, "mapping", &[], &[(2, v(0.3, 0.5, 0.2)), (3, v(2.0, 1.0, 1.5)), (1, v(0.1, 0.0, 0.2))]);
                g.connect_output(coords, 3, mapping, Socket::Input(0));
                let checker = add(g, "checker_texture", &[], &[(3, f(2.0))]);
                g.connect_output(mapping, 0, checker, Socket::Input(0));
                (checker, 1)
            })),
            ("ramp-ease", Box::new(|g| ramp(g, tequila_texture::gradient::Interpolation::Ease))),
            ("ramp-constant", Box::new(|g| ramp(g, tequila_texture::gradient::Interpolation::Constant))),
            ("map-range-stepped", Box::new(|g| {
                let wave = add(g, "wave_texture", &[], &[(1, f(2.0))]);
                let steps = add(g, "map_range", &[("interpolation", Choice("stepped"))], &[(5, f(3.0))]);
                g.connect_output(wave, 1, steps, Socket::Input(0));
                let clamp = add(g, "clamp", &[("range", Bool(true))], &[(1, f(0.8)), (2, f(0.2))]);
                g.connect_output(steps, 0, clamp, Socket::Input(0));
                (clamp, 0)
            })),
        ];
        let mut wrong = Vec::new();
        for (name, build) in &cases {
            let [tequila, cycles] = both_engines(&format!("node-{name}"), |scene| {
                let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
                let (node, output) = build(&mut graph);
                let solo = graph.soloed_at(node, output).expect("a value to show");
                let program = solo.compile().expect("compiles");
                scene.materials[0] = solo.material().clone();
                *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                    .with_programs(vec![Some(Arc::new(program))], Arc::new(tequila_render_core::shade::TextureTable::new()));
            });
            let (a, b) = (inside_the_sphere(&tequila), inside_the_sphere(&cycles));
            let ((ma, sa), (mb, sb)) = (mean_and_spread(&a), mean_and_spread(&b));
            let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
            for (x, y) in a.iter().zip(&b) {
                cov += (x - ma) * (y - mb);
                va += (x - ma) * (x - ma);
                vb += (y - mb) * (y - mb);
            }
            // A flat field correlates with nothing: there it is the level
            // alone that has to agree.
            let correlation = if sa < 2.0 && sb < 2.0 { 1.0 } else { cov / (va.sqrt() * vb.sqrt()).max(1e-9) };
            let (fa, fb) = (channel_means(&tequila), channel_means(&cycles));
            let channels_agree = (0..3).all(|c| (fa[c] - fb[c]).abs() < 4.0 + 0.04 * fa[c].max(fb[c]));
            eprintln!("{name}: correlation {correlation:.3}, Tequila {fa:.1?} spread {sa:.1}, Cycles {fb:.1?} spread {sb:.1}");
            if correlation < 0.97 || !channels_agree {
                wrong.push(format!("{name}: correlation {correlation:.3}, Tequila {fa:.1?}, Cycles {fb:.1?}"));
            }
        }
        assert!(wrong.is_empty(), "these Blender nodes shade differently in the two engines:\n{}", wrong.join("\n"));
    }

    /// Tequila's exact patterns - a perforated mesh, round and hexagonal,
    /// a twill, contour lines - land on the same pixels in Cycles, seed and
    /// all. Cycles read each as a flat grey. Skipped where Cycles is not
    /// installed.
    #[test]
    fn exact_patterns_land_where_tequila_puts_them() {
        use tequila_core::spectrum::Spectrum;
        use tequila_texture::gradient::Gradient;
        use tequila_texture::procedural::{HoleShape, Procedural, WeavePattern};
        use tequila_texture::projection::Axis;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let mut wrong = Vec::new();
        for (name, procedural) in [
            ("mesh", Procedural::Mesh { shape: HoleShape::Round, frequency: 3.0, size: 0.6, softness: 0.1 }),
            ("hex", Procedural::Mesh { shape: HoleShape::Hex, frequency: 3.0, size: 0.6, softness: 0.1 }),
            ("twill", Procedural::Weave { pattern: WeavePattern::Twill, frequency: 2.5, width: 0.85, variance: 0.0 }),
            ("contour", Procedural::Contour { axis: Axis::Y, frequency: 3.0, width: 0.3, softness: 0.2, distortion: 0.0 }),
        ] {
            let [tequila, cycles] = both_engines(name, |scene| {
                procedural_material(scene, procedural.clone(), Gradient::two(Spectrum::splat(0.02), Spectrum::splat(0.9)));
            });
            let (a, b) = (inside_the_sphere(&tequila), inside_the_sphere(&cycles));
            let ((ma, sa), (mb, _)) = (mean_and_spread(&a), mean_and_spread(&b));
            let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
            for (x, y) in a.iter().zip(&b) {
                cov += (x - ma) * (y - mb);
                va += (x - ma) * (x - ma);
                vb += (y - mb) * (y - mb);
            }
            let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
            eprintln!("{name}: correlation {correlation:.3}, mean {ma:.1} vs {mb:.1}, spread {sa:.1}");
            if sa < 25.0 || correlation < 0.9 || (mb / ma - 1.0).abs() > 0.1 {
                wrong.push(format!("{name}: correlation {correlation:.2}, Tequila {ma:.1} bright (spread {sa:.1}), Cycles {mb:.1}"));
            }
        }
        assert!(wrong.is_empty(), "patterns that differ: {wrong:#?}");
    }

    /// Tequila's random patterns come out of Cycles as Tequila's renderer
    /// makes them: not the same features - the hashes that place them differ
    /// - but as bright on average and as contrasty, read through a
    /// black-to-white ramp. Cycles read each as a flat grey. Skipped where
    /// Cycles is not installed.
    #[test]
    fn random_patterns_have_tequila_contrast_in_cycles() {
        use tequila_core::spectrum::Spectrum;
        use tequila_texture::gradient::Gradient;
        use tequila_texture::noise::Fractal;
        use tequila_texture::procedural::{Procedural, WeavePattern};
        use tequila_texture::projection::Axis;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let mut wrong = Vec::new();
        for (name, procedural) in [
            ("ridged", Procedural::Ridged { fractal: Fractal { octaves: 4, ..Fractal::default() }, frequency: 4.0 }),
            ("granite", Procedural::Granite { frequency: 6.0, speckle: 0.3, fine: 0.3 }),
            ("leather", Procedural::Leather { frequency: 8.0, crack_width: 0.12, wrinkle: 0.25 }),
            ("spots", Procedural::Spots { frequency: 6.0, coverage: 0.5, variance: 0.3, softness: 0.2 }),
            ("camouflage", Procedural::Camouflage { frequency: 4.0, layers: 4 }),
            ("radial", Procedural::BrushedRadial { axis: Axis::Z, frequency: 30.0, stretch: 10.0 }),
            ("scratches", Procedural::Scratches { frequency: 5.0, width: 0.08, density: 0.7, spread: 0.3 }),
            ("cloth", Procedural::Weave { pattern: WeavePattern::Plain, frequency: 3.0, width: 0.8, variance: 0.5 }),
            (
                "planks",
                Procedural::WoodAdvanced {
                    axis: Axis::Y,
                    frequency: 6.0,
                    distortion: 0.1,
                    grain: 0.2,
                    fractal: Fractal::default(),
                    ring_variation: 0.4,
                    latewood: 0.3,
                    contrast: 0.7,
                    flecks: 0.5,
                    knots: 0.5,
                    board_width: 0.4,
                },
            ),
        ] {
            let [tequila, cycles] = both_engines(name, |scene| {
                procedural_material(scene, procedural.clone(), Gradient::two(Spectrum::splat(0.02), Spectrum::splat(0.9)));
            });
            let ((ma, sa), (mb, sb)) = (mean_and_spread(&inside_the_sphere(&tequila)), mean_and_spread(&inside_the_sphere(&cycles)));
            eprintln!("{name}: mean {ma:.1} vs {mb:.1}, spread {sa:.1} vs {sb:.1}");
            if sa < 15.0 || (mb / ma - 1.0).abs() > 0.12 || (sb / sa - 1.0).abs() > 0.25 {
                wrong.push(format!("{name}: Tequila {ma:.1} bright, spread {sa:.1}; Cycles {mb:.1}, spread {sb:.1}"));
            }
        }
        assert!(wrong.is_empty(), "patterns that differ: {wrong:#?}");
    }

    /// A procedural height bumps the surface in Cycles as in Tequila's
    /// renderer: smooth bands, `depth` deep, round the sphere, under a lamp
    /// overhead, darken and lighten the same pixels by as much in both -
    /// measured against each engine's own unbumped sphere, so only what the
    /// bump does is compared, and through a clear coat, which tilts with
    /// the bump in Tequila's. Cycles left height bumps out. Skipped where
    /// Cycles is not installed.
    #[test]
    fn a_procedural_bump_tilts_the_surface_as_in_tequila() {
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket};
        use tequila_materials::material_type::{MaterialType, Param};
        use tequila_render_core::light::{Light, LightKind, LightSet, PointLight};
        use tequila_texture::noise::Fractal;
        use tequila_texture::procedural::Procedural;
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let bumped = |name: &str, depth: f32| {
            both_engines(name, move |scene| {
                let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
                graph.material_mut().coat_weight = 1.0;
                graph.material_mut().coat_roughness = 0.2;
                // Bands with no wander: sin(y * f) / 2 + 1/2.
                let bands = Procedural::Marble { axis: tequila_texture::projection::Axis::Y, frequency: 10.0, distortion: 0.0, fractal: Fractal::default() };
                let pattern = graph.add_node(Node::new(NodeKind::Procedural { procedural: Box::new(bands), seed: 7 }));
                let bump = graph.add_node(Node::new(NodeKind::Bump { depth }));
                graph.connect(pattern, bump, Socket::Input(0));
                let output = graph.output();
                graph.connect(bump, output, Socket::Param(Param::Bump));
                let program = graph.compile().expect("compiles");
                scene.materials[0] = graph.material().clone();
                let lamp = Light::new(LightKind::Point(PointLight {
                    position: vec3(1.0, 4.5, 1.5),
                    intensity: tequila_core::spectrum::Spectrum::splat(40.0),
                    radius: 0.05,
                }));
                *scene = std::mem::replace(scene, tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2))
                    .with_programs(vec![Some(Arc::new(program))], Arc::new(tequila_render_core::shade::TextureTable::new()))
                    .with_lights(Arc::new(LightSet::new(vec![lamp])));
            })
        };
        let [tequila_flat, cycles_flat] = bumped("bump-flat", 0.0);
        let [tequila_bumped, cycles_bumped] = bumped("bump-bands", 0.1);
        let change = |bumped: &[u8], flat: &[u8]| -> Vec<f64> {
            inside_the_sphere(bumped).iter().zip(inside_the_sphere(flat)).map(|(a, b)| a - b).collect()
        };
        let (a, b) = (change(&tequila_bumped, &tequila_flat), change(&cycles_bumped, &cycles_flat));
        let ((ma, sa), (mb, sb)) = (mean_and_spread(&a), mean_and_spread(&b));
        assert!(sa > 12.0, "the bump changed nothing in Tequila's render: spread {sa:.1}");
        let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(&b) {
            cov += (x - ma) * (y - mb);
            va += (x - ma) * (x - ma);
            vb += (y - mb) * (y - mb);
        }
        let correlation = cov / (va.sqrt() * vb.sqrt()).max(1e-9);
        assert!(correlation > 0.8, "the bump changed the two engines' renders differently: correlation {correlation:.2}");
        assert!((sb / sa - 1.0).abs() < 0.25, "the bump changed Tequila's render by {sa:.1}, Cycles' by {sb:.1}");
    }

    /// Cycles on the graphics card is the default engine, and what Cycles
    /// cannot make is Tequila's own renderer's: a render with layers goes to
    /// it whichever engine is picked, and one asking for light groups does
    /// only where the Cycles library predates passes.
    #[test]
    fn cycles_is_the_default_and_hands_on_what_it_cannot_make() {
        assert_eq!(RenderSettings::default().engine, RenderEngine::CyclesGpu);
        for engine in RenderEngine::ALL {
            let relit = RenderSettings { engine, relight: true, ..RenderSettings::default() };
            let by_tequila = engine == RenderEngine::Tequila || !tequila_render_cycles::has_passes() || cycles_here().is_err();
            assert_eq!(engine_for(&relit, false).0 == RenderEngine::Tequila, by_tequila, "{engine:?} with light groups");
            let plain = RenderSettings { engine, ..RenderSettings::default() };
            assert_eq!(engine_for(&plain, true).0, RenderEngine::Tequila, "{engine:?} with layers");
        }
        let tequila = RenderSettings { engine: RenderEngine::Tequila, ..RenderSettings::default() };
        assert_eq!(engine_for(&tequila, false), (RenderEngine::Tequila, None));
        // And otherwise Cycles, on the card where there is one.
        let expected = match cycles_here() {
            Err(_) => RenderEngine::Tequila,
            Ok(true) => RenderEngine::CyclesGpu,
            Ok(false) => RenderEngine::CyclesCpu,
        };
        assert_eq!(engine_for(&RenderSettings::default(), false).0, expected);
    }

    fn job(id: u64, directory: &std::path::Path, settings: RenderSettings) -> BatchJob {
        BatchJob {
            layers: Vec::new(),
            pending_environment: None,
            id,
            name: "Test".into(),
            scene: Arc::new(tequila_scene::demo::demo_scene(
                tequila_scene::demo::DemoMaterial::Plastic,
                0.2,
            )),
            camera: Camera::look_at(vec3(2.0, 1.5, 3.0), vec3(0.0, 0.5, 0.0), Vec3::Y),
            settings: RenderSettings {
                output_directory: directory.to_string_lossy().to_string(),
                ..settings
            },
            config: IntegratorConfig::performance(),
            style: ImageStyle::default(),
            display: DisplayPipeline::default(),
            object_names: vec!["Sphere".into(), "Ground".into()],
            material_names: vec!["Plastic".into(), "Matte".into()],
            fields: NameFields {
                project: "Test".into(),
                camera: "Hero".into(),
                date: "2026-01-01".into(),
                ..Default::default()
            },
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// A wireframe job is drawn, not rendered, and written as a render is:
    /// its file at its size, the ground the colour it was given, and the
    /// edges across it - the ground under the demo's sphere left out.
    #[test]
    fn a_wireframe_job_draws_the_edges_in_the_colours_it_was_given() {
        use tequila_scene::render::{RenderKind, WireframeSettings};
        let directory = scratch("tequila-batch-wireframe");
        let renderer = BatchRenderer::new();
        let settings = RenderSettings {
            kind: RenderKind::Wireframe,
            width: 160,
            height: 100,
            wireframe: WireframeSettings::LIGHT,
            filename_template: "{project}_{camera}".into(),
            ..Default::default()
        };
        renderer.submit(job(7, &directory, settings));
        let (path, samples) = match wait_for_finish(&renderer) {
            BatchEvent::Finished { path, samples, .. } => (path, samples),
            BatchEvent::Failed { message, .. } => panic!("the wireframe failed: {message}"),
            _ => unreachable!(),
        };
        assert_eq!(samples, 0, "a drawing takes no samples");
        let image = tequila_texture::image::Image::load_raw(&path).expect("the drawing reads");
        assert_eq!((image.width, image.height), (160, 100));
        let pixel = |x: usize, y: usize| {
            let i = (y * image.width + x) * image.channels;
            [image.data[i], image.data[i + 1], image.data[i + 2]]
        };
        assert!(pixel(0, 0).iter().all(|c| (c - 1.0).abs() < 1.0 / 255.0), "paper white where nothing is: {:?}", pixel(0, 0));
        let inked = (0..image.height).flat_map(|y| (0..image.width).map(move |x| (x, y))).filter(|&(x, y)| pixel(x, y)[0] < 0.5).count();
        assert!(inked > 200, "the sphere's edges are drawn: {inked} dark pixels");
        assert!(inked < 160 * 100 / 2, "and not the ground's: {inked} dark pixels");
    }

    fn wait_for_finish(renderer: &BatchRenderer) -> BatchEvent {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            for event in renderer.drain() {
                match event {
                    BatchEvent::Finished { .. } | BatchEvent::Failed { .. } => return event,
                    _ => {}
                }
            }
            assert!(Instant::now() < deadline, "the batch renderer never finished");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A render kept for relighting comes back split by light group, and
    /// the groups at their own strength recombine into the render itself;
    /// turning one off takes away exactly its light.
    #[test]
    fn a_relightable_render_recombines_into_itself() {
        use tequila_core::spectrum::Spectrum;
        use tequila_render_core::light::{Light, LightKind, LightSet, PointLight};
        let directory = scratch("tequila-batch-relight");
        let renderer = BatchRenderer::new();
        let mut relit = job(
            4,
            &directory,
            RenderSettings {
                width: 32,
                height: 24,
                max_samples: 16,
                // Off, so the check is exact: the image is the mean of the
                // samples and the groups are the same samples, split.
                denoise: false,
                relight: true,
                filename_template: "{project}_{camera}".into(),
                ..Default::default()
            },
        );
        let lamp = Light::new(LightKind::Point(PointLight {
            position: vec3(1.5, 2.5, 2.0),
            intensity: Spectrum::splat(6.0),
            radius: 0.1,
        }))
        .with_layer(2);
        relit.scene = Arc::new((*relit.scene).clone().with_lights(Arc::new(LightSet::new(vec![lamp]))));
        renderer.submit(relit);

        let data = match wait_for_finish(&renderer) {
            BatchEvent::Finished { relight: Some(data), .. } => data,
            BatchEvent::Finished { relight: None, .. } => panic!("the render kept no light groups"),
            BatchEvent::Failed { message, .. } => panic!("the job failed: {message}"),
            _ => unreachable!(),
        };
        let names: Vec<&str> = data.groups.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"LightLayer2") && names.contains(&"Environment"), "{names:?}");

        // The saved render is what the groups at full strength develop into.
        let ones = vec![Spectrum::ONE; data.groups.len()];
        let same = data.compose(&ones, usize::MAX);
        let written = tequila_texture::image::Image::load_raw(&data.path).expect("the render reads");
        let rgba = same.to_rgba8();
        let mut worst = 0i32;
        for (i, pixel) in rgba.chunks(4).enumerate() {
            for c in 0..3 {
                let file = (written.data[i * written.channels + c] * 255.0).round() as i32;
                worst = worst.max((file - pixel[c] as i32).abs());
            }
        }
        assert!(worst <= 2, "the recombined groups differ from the render by {worst} of 255");

        // Turning the lamp's group off takes away exactly its light.
        let lamp_index = names.iter().position(|n| *n == "LightLayer2").unwrap();
        let mut off = ones.clone();
        off[lamp_index] = Spectrum::ZERO;
        let without = data.compose(&off, usize::MAX);
        let lamp_only: f32 = data.groups[lamp_index].1.iter().map(|s| s.luminance()).sum();
        let total: f32 = same.color.iter().map(|s| s.luminance()).sum();
        let rest: f32 = without.color.iter().map(|s| s.luminance()).sum();
        assert!(lamp_only > 0.0, "the lamp's group is empty");
        assert!((total - rest - lamp_only).abs() < total * 1e-4, "turning the lamp off took away {} of its {lamp_only}", total - rest);

        // And the full-size relit copy is saved beside the render.
        let saved = data.save(&off).expect("saves");
        assert!(saved.is_file() && saved.to_string_lossy().contains("_relit"), "{}", saved.display());
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A PSD with passes gets the data passes a compositor reaches for, the
    /// labels with their own transparency. It had the lighting passes only:
    /// occlusion, shadow and labels were EXR-only.
    #[test]
    fn a_psd_gets_occlusion_shadow_and_labels() {
        use tequila_core::spectrum::Spectrum;
        let set = AovSet::standard();
        let mut aovs = AovBuffers::new(2, 2, &set);
        let result = tequila_render_core::integrator::PathResult {
            radiance: Spectrum::splat(0.5),
            coverage: 1.0,
            labels: Spectrum::splat(0.3),
            labels_alpha: 0.5,
            ..Default::default()
        };
        let passes = tequila_render_core::aov::PathPasses::new(set.light_paths.len());
        for y in 0..2 {
            for x in 0..2 {
                aovs.accumulate(x, y, &result, &passes);
            }
        }
        let settings = RenderSettings { width: 2, height: 2, ..Default::default() };
        let layers = layers_of(Some(&aovs), &set, 1, &settings);
        let named = |name: &str| layers.iter().find(|(n, _, _)| n == name);
        assert!(named("AmbientOcclusion").is_some() && named("Shadow").is_some(), "no data passes");
        let (_, colour, alpha) = named("Labels").expect("no Labels layer");
        let alpha = alpha.as_ref().expect("the labels layer has no transparency of its own");
        assert!((alpha[0] - 0.5).abs() < 1e-5, "{alpha:?}");
        // Straight colour: the premultiplied 0.3 over coverage 0.5.
        assert!((colour[0].g - 0.6).abs() < 1e-4, "{:?}", colour[0]);
    }

    /// Each enabled render layer is rendered and written beside the main
    /// image, named after it, holding only what the layer's camera sees.
    #[test]
    fn a_render_layer_is_written_beside_the_image() {
        let directory = scratch("tequila-batch-layer");
        let renderer = BatchRenderer::new();
        let mut layered = job(
            3,
            &directory,
            RenderSettings {
                width: 48,
                height: 32,
                max_samples: 8,
                filename_template: "{project}_{camera}".into(),
                ..Default::default()
            },
        );
        // A layer with nothing in it: every instance hidden from the camera,
        // so its image is empty — alpha zero throughout — while the main one
        // shows the sphere.
        let mut hidden = (*layered.scene).clone();
        for instance in &mut hidden.instances {
            instance.visible_to_camera = false;
        }
        layered.layers = vec![("Sphere only".into(), Arc::new(hidden))];
        renderer.submit(layered);

        let path = match wait_for_finish(&renderer) {
            BatchEvent::Finished { path, .. } => path,
            BatchEvent::Failed { message, .. } => panic!("the job failed: {message}"),
            _ => unreachable!(),
        };
        let layer = directory.join("Test_Hero_Sphere_only.png");
        assert!(layer.is_file(), "no layer image beside {path}");
        let image = tequila_texture::image::Image::load_png(&layer).expect("the layer reads");
        assert_eq!((image.width, image.height), (48, 32));
        let covered = image.data.chunks(image.channels).filter(|p| p[3] > 0.01).count();
        assert_eq!(covered, 0, "a layer with no members covered {covered} pixels");
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A queued render produces a file, on its own thread, without anyone
    /// waiting on it.
    #[test]
    fn a_job_renders_and_writes_its_file() {
        let directory = scratch("tequila-batch-file");
        let renderer = BatchRenderer::new();
        renderer.submit(job(
            1,
            &directory,
            RenderSettings {
                width: 64,
                height: 48,
                max_samples: 16,
                filename_template: "{project}_{camera}".into(),
                ..Default::default()
            },
        ));

        match wait_for_finish(&renderer) {
            BatchEvent::Finished {
                id,
                path,
                samples,
                halt,
                preview,
                ..
            } => {
                assert_eq!(id, 1);
                assert!(path.ends_with("Test_Hero.png"), "{path}");
                assert!(std::path::Path::new(&path).is_file());
                assert_eq!(samples, 16);
                assert_eq!(halt, Halt::Samples);
                assert_eq!(preview.len(), 64 * 48 * 4);
            }
            BatchEvent::Failed { message, .. } => panic!("the job failed: {message}"),
            _ => unreachable!(),
        }
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The time limit stops a render that would otherwise run to its sample
    /// count. This is the half of §1.8's halting that a queue depends on.
    #[test]
    fn a_time_limit_stops_a_long_render() {
        let directory = scratch("tequila-batch-time");
        let renderer = BatchRenderer::new();
        renderer.submit(job(
            2,
            &directory,
            RenderSettings {
                width: 96,
                height: 96,
                // Far more than the time allows.
                max_samples: 100_000,
                max_seconds: Some(1),
                ..Default::default()
            },
        ));

        match wait_for_finish(&renderer) {
            BatchEvent::Finished {
                samples,
                seconds,
                halt,
                ..
            } => {
                assert_eq!(halt, Halt::Time);
                assert!(samples < 100_000, "it rendered everything anyway");
                assert!(samples > 0, "it rendered nothing at all");
                assert!(seconds < 30.0, "it overran its one-second limit by {seconds}s");
            }
            BatchEvent::Failed { message, .. } => panic!("the job failed: {message}"),
            _ => unreachable!(),
        }
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Sidecars are written beside the image, with the same stem, and the XMP
    /// is a real packet.
    #[test]
    fn sidecars_land_beside_the_image() {
        let directory = scratch("tequila-batch-sidecars");
        let renderer = BatchRenderer::new();
        renderer.submit(job(
            3,
            &directory,
            RenderSettings {
                width: 32,
                height: 32,
                max_samples: 4,
                write_xmp: true,
                write_text_metadata: true,
                filename_template: "shot".into(),
                ..Default::default()
            },
        ));

        match wait_for_finish(&renderer) {
            BatchEvent::Finished { path, .. } => {
                let image = std::path::Path::new(&path);
                let xmp = image.with_extension("xmp");
                let text = image.with_extension("txt");
                assert!(xmp.is_file(), "no XMP beside {path}");
                assert!(text.is_file(), "no text sidecar beside {path}");
                let xmp = std::fs::read_to_string(&xmp).unwrap();
                assert!(xmp.contains("x:xmpmeta"));
                // The bias a user should be told about rather than discover.
                assert!(xmp.contains("FireflyRejection"), "{xmp}");
                let text = std::fs::read_to_string(&text).unwrap();
                assert!(text.contains("Samples: 4"), "{text}");
                assert!(text.contains("StoppedBecause"), "{text}");
            }
            BatchEvent::Failed { message, .. } => panic!("the job failed: {message}"),
            _ => unreachable!(),
        }
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A queued render is denoised for the *file* and never into the
    /// accumulation buffer (§13). The observable half of that is simply that
    /// denoising happens at all: a setting that reached the panel and not the
    /// renderer would look like a denoiser that does nothing.
    #[test]
    fn a_queued_render_is_denoised() {
        let directory = scratch("tequila-batch-denoise");
        let renderer = BatchRenderer::new();
        let base = RenderSettings {
            width: 96,
            height: 72,
            // Few enough samples that the noise is unmistakable.
            max_samples: 4,
            filename_template: "noisy".into(),
            denoise: false,
            ..Default::default()
        };
        renderer.submit(job(10, &directory, base.clone()));
        let noisy = match wait_for_finish(&renderer) {
            BatchEvent::Finished { preview, .. } => preview,
            BatchEvent::Failed { message, .. } => panic!("{message}"),
            _ => unreachable!(),
        };

        renderer.submit(job(
            11,
            &directory,
            RenderSettings {
                denoise: true,
                filename_template: "clean".into(),
                ..base
            },
        ));
        let clean = match wait_for_finish(&renderer) {
            BatchEvent::Finished { preview, .. } => preview,
            BatchEvent::Failed { message, .. } => panic!("{message}"),
            _ => unreachable!(),
        };

        // Local variation, which is what noise is. A denoised image has less of
        // it; comparing the *means* would compare exposure instead and pass
        // whatever the denoiser did.
        let roughness = |pixels: &[u8]| -> f64 {
            let mut total = 0.0;
            for y in 1..71usize {
                for x in 1..95usize {
                    let at = |dx: i32, dy: i32| {
                        let index = (((y as i32 + dy) * 96 + (x as i32 + dx)) * 4) as usize;
                        pixels[index] as f64
                    };
                    total += (at(0, 0) - at(1, 0)).abs() + (at(0, 0) - at(0, 1)).abs();
                }
            }
            total
        };

        let before = roughness(&noisy);
        let after = roughness(&clean);
        // A denoiser may be absent on a machine without the library, in which
        // case both renders are identical and there is nothing to claim.
        if (before - after).abs() < 1e-9 {
            eprintln!("no denoiser available; skipping the comparison");
            let _ = std::fs::remove_dir_all(&directory);
            return;
        }
        assert!(
            after < before * 0.9,
            "denoising left {after} of variation against {before} without it"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A render asking for passes in EXR gets the multi-layer file, with the
    /// Cryptomatte channels the queue is the only way to produce.
    #[test]
    fn a_queued_exr_carries_its_passes() {
        let directory = scratch("tequila-batch-passes");
        let renderer = BatchRenderer::new();
        renderer.submit(job(
            4,
            &directory,
            RenderSettings {
                width: 48,
                height: 32,
                max_samples: 8,
                format: OutputFormat::Exr,
                write_passes: true,
                filename_template: "passes".into(),
                ..Default::default()
            },
        ));

        match wait_for_finish(&renderer) {
            BatchEvent::Finished { path, .. } => {
                assert!(path.ends_with("passes.exr"), "{path}");
                let read = exr::prelude::read_all_flat_layers_from_file(&path)
                    .expect("the file reads back");
                let names: Vec<String> = read.layer_data[0]
                    .channel_data
                    .list
                    .iter()
                    .map(|c| c.name.to_string())
                    .collect();
                for expected in ["R", "Diffuse.R", "CryptoObject00.R", "Depth"] {
                    assert!(names.contains(&expected.to_string()), "{names:?}");
                }
                // Its alpha is the finished image's, opaque in the default
                // mode, not the framebuffer's coverage with the background
                // left out.
                let alpha = read.layer_data[0].channel_data.list.iter().find(|c| c.name.to_string() == "A").expect("an alpha channel");
                assert!(alpha.sample_data.values_as_f32().all(|a| (a - 1.0).abs() < 1e-3), "the beauty's alpha is the raw coverage");
            }
            BatchEvent::Failed { message, .. } => panic!("the job failed: {message}"),
            _ => unreachable!(),
        }
        let _ = std::fs::remove_dir_all(&directory);
    }
}
