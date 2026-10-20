//! The render orchestrator.
//!
//! `docs/01-RENDERER-CORE.md` §9.1 states the contract this module exists to
//! enforce:
//!
//! > The UI thread **never** waits on a render. It draws the most recent
//! > completed display texture. If none is ready, it redraws the previous one.
//! > Frame pacing is decoupled from sample production.
//!
//! So rendering happens on its own thread, and the two communicate through a
//! mutex holding the latest finished image plus an atomic generation counter.
//! The UI takes that lock only to clone a small buffer — never while a render
//! is in flight.

use tequila_color::{DisplayPipeline, ImageStyle};
use tequila_core::math::Ray;
use tequila_core::math::Vec3;
use tequila_denoise::{DenoiseSettings, Denoiser, Quality};
use tequila_render_core::camera::Camera;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::intersect::{Hit, Intersector};
use tequila_render_core::scene::SceneSnapshot;
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{preview_pass, render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// What the viewport is showing.
///
/// `TEQUILA_SPEC.md` §1.1 makes the path tracer the viewport, and [`Rendered`]
/// stays the default and the only mode anything is ever shipped from. [`Fast`]
/// exists for the part of the work that is not look development — placing
/// parts, aiming a camera, finding which bracket is which — where an instant
/// picture beats a correct one and the fans should stay quiet. The HUD says
/// which is on, always, because the danger of a preview mode is someone
/// judging a material in it.
///
/// [`Rendered`]: Shading::Rendered
/// [`Fast`]: Shading::Fast
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shading {
    Rendered,
    #[default]
    Fast,
}

impl Shading {

    /// Where a fast preview stops adding samples.
    ///
    /// It has no random component beyond the pixel jitter, so it stops
    /// improving once the edges are smooth — and then the render thread must
    /// actually go to sleep. Continuing to trace an image that cannot change is
    /// the single most expensive way to display a static picture, and the whole
    /// reason someone reaches for this mode is that the machine was busy.
    pub const FAST_SAMPLE_CAP: u32 = 4;
}

/// What the UI can change about the render.
#[derive(Clone, Debug)]
pub struct RenderRequest {
    pub camera: Camera,
    pub width: usize,
    pub height: usize,
    /// What the integrator runs with, from the Lighting tab.
    ///
    /// The configuration itself rather than the preset name: §1.6's Custom is
    /// the absence of a preset, so a name cannot describe every state the tab
    /// can be in — and the renderer has no business knowing which preset a
    /// configuration came from anyway.
    pub config: IntegratorConfig,
    /// The scene to trace, flattened from the document by
    /// [`tequila_scene::snapshot`].
    ///
    /// Compared by pointer, not by content: comparing two 4,000-instance
    /// snapshots field by field on every frame would cost more than the render
    /// it is trying to avoid, and the app produces a new `Arc` exactly when
    /// the document changes.
    pub scene: Arc<SceneSnapshot>,
    /// Bumped by the app whenever it builds a new scene, so an unchanged
    /// document does not restart the render.
    pub scene_generation: u64,
    pub denoise: bool,
    pub denoise_strength: f32,
    /// Passes between denoises once the camera has settled. One is §9.2's own
    /// decaying policy; larger is a fixed interval the user asked for.
    pub denoise_cadence: u32,
    /// Filter the guides before denoising. `None` leaves it to the sample
    /// count, which is the right call until someone disagrees.
    pub prefilter_guides: Option<bool>,
    /// The document's firefly rejection and indirect clamp, as the final
    /// render applies them. The viewport always rejected and never clamped,
    /// so switching either off changed the saved image and not the preview.
    pub reject_fireflies: bool,
    pub clamp_sample: Option<f32>,
    /// Set while the user is dragging, which selects the interactive regime.
    pub interacting: bool,
    /// Solid is being drawn by the GPU ([`crate::solid_gpu`]): build the
    /// scene, for picking, but trace nothing.
    pub gpu_solid: bool,
    /// Path traced, or [`Shading::Fast`].
    pub shading: Shading,
    /// Which path tracer draws a Rendered viewport: Tequila's own, or Cycles
    /// ([`crate::cycles_view`]). The final render's choice, so the preview is
    /// the render it previews.
    pub engine: tequila_scene::render::RenderEngine,
    /// The render's pixels per point of viewport: one on a standard screen,
    /// the screen's own on a Retina one (`render_scale` in `main.rs`).
    /// `width` and `height` are pixels, which Cycles renders; Tequila's own
    /// renderer traces the points ([`RenderRequest::traced_size`]).
    pub pixels_per_point: f32,
}

impl RenderRequest {
    /// The size Tequila's own renderer traces: a pixel per point, as it
    /// always has, stretched over a Retina viewport. The screen's own pixels
    /// there were three and a half times the samples and the denoise, and
    /// the processor's denoiser took 1.7 s over each picture, during which
    /// the view could not begin to move (`retina_viewport_benchmark`).
    fn traced_size(&self) -> (usize, usize) {
        let points = |n: usize| ((n as f32 / self.pixels_per_point).round() as usize).max(1);
        (points(self.width), points(self.height))
    }
}

/// A finished image, ready to upload.
#[derive(Clone, Debug, Default)]
pub struct DisplayImage {
    pub width: usize,
    pub height: usize,
    /// Tightly packed RGBA8.
    pub pixels: Vec<u8>,
    pub samples: u32,
    pub converged_fraction: f32,
    pub samples_per_second: f64,
    pub triangles: usize,
    pub instances: usize,
    pub milliseconds_to_first_pixel: f64,
    pub denoised: bool,
    /// Seconds since accumulation last restarted, which is what the HUD's
    /// elapsed readout means — not since the application started.
    pub elapsed_seconds: f64,
    /// Linear resolution divisor. Above 1 means the viewport is showing an
    /// upsampled reduced-resolution render, which the HUD must say rather than
    /// leave the user wondering why the image looks soft.
    pub resolution_divisor: u32,
    /// Which mode produced this image. The HUD says so, and it is read from
    /// the image rather than from the app's own setting so it can never
    /// describe a frame that was made before the mode changed.
    pub shading: Shading,
}

/// Equality decides whether a submitted request restarts the render, so it
/// compares the scene by generation rather than by content. Two requests with
/// the same generation describe the same scene by construction — the app bumps
/// it on every document change — and comparing thousands of instances instead
/// would cost more than the restart it avoids.
impl PartialEq for RenderRequest {
    fn eq(&self, other: &Self) -> bool {
        self.camera == other.camera
            && self.width == other.width
            && self.height == other.height
            && self.config == other.config
            && self.scene_generation == other.scene_generation
            && self.denoise == other.denoise
            && self.denoise_strength == other.denoise_strength
            && self.denoise_cadence == other.denoise_cadence
            && self.prefilter_guides == other.prefilter_guides
            && self.reject_fireflies == other.reject_fireflies
            && self.clamp_sample == other.clamp_sample
            && self.shading == other.shading
            && self.interacting == other.interacting
            && self.engine == other.engine
            && self.pixels_per_point == other.pixels_per_point
    }
}

/// What the viewport says about its engine beyond the frame: Cycles getting
/// ready before it has shown anything, or why it is not being used.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineNote {
    /// Cycles has shown nothing yet; what it says it is doing. The first
    /// render on a graphics card compiles its code, which can take minutes,
    /// and a black view that says nothing reads as a broken one.
    Preparing(String),
    /// Cycles on `engine` could not render, so the viewport uses Tequila's
    /// own renderer, on the processor: why, in Cycles' words.
    FellBack { engine: tequila_scene::render::RenderEngine, why: String },
}

struct Shared {
    request: Mutex<Option<RenderRequest>>,
    /// See [`EngineNote`]; `None` when there is nothing to say.
    engine_note: Mutex<Option<EngineNote>>,
    /// The intersector the render thread most recently built, shared so the UI
    /// can pick against exactly the geometry that is on screen.
    ///
    /// Picking through the same BVH the renderer uses is what stops the
    /// viewport and the Scene tab from disagreeing about what was clicked —
    /// a second, UI-only acceleration structure would be a second thing that
    /// can be out of date.
    intersector: Mutex<Option<Arc<EmbreeIntersector>>>,
    /// Bumped whenever the request changes, so the render thread notices
    /// without polling the request itself.
    request_generation: AtomicU64,
    image: Mutex<DisplayImage>,
    image_generation: AtomicU64,
    /// Set by the UI so a style change can re-tonemap without re-rendering.
    style: Mutex<ImageStyle>,
    /// And what turns the graded values into display code values: the built-in
    /// transform, or an OCIO pipeline. Alongside the style rather than inside
    /// it, because it is a compiled object holding LUT tables and the style is
    /// a small block of numbers the document copies on every edit.
    display: Mutex<DisplayPipeline>,
    /// Which instances are selected, for the outline.
    ///
    /// Its own channel rather than a field of the request, and that is the
    /// whole design decision: request equality is what decides whether the
    /// render restarts, so a selection carried in the request would throw away
    /// every accumulated sample each time somebody clicked a part. It rides
    /// with the style instead, on the `Dirty::Display` path of §9.3 — a
    /// re-develop of what is already there.
    selection: Mutex<Vec<u32>>,
    style_generation: AtomicU64,
    running: AtomicBool,
    paused: AtomicBool,
    parked: AtomicBool,
    wakeup: Condvar,
    wakeup_lock: Mutex<()>,
}

/// Owns the render thread.
#[derive(Debug)]
pub struct Orchestrator {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

impl Orchestrator {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            request: Mutex::new(None),
            engine_note: Mutex::new(None),
            intersector: Mutex::new(None),
            request_generation: AtomicU64::new(0),
            image: Mutex::new(DisplayImage::default()),
            image_generation: AtomicU64::new(0),
            style: Mutex::new(ImageStyle::default()),
            display: Mutex::new(DisplayPipeline::default()),
            selection: Mutex::new(Vec::new()),
            style_generation: AtomicU64::new(0),
            running: AtomicBool::new(true),
            paused: AtomicBool::new(false),
            parked: AtomicBool::new(false),
            wakeup: Condvar::new(),
            wakeup_lock: Mutex::new(()),
        });

        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("tequila-render".into())
            .spawn(move || render_loop(worker))
            .expect("failed to spawn the render thread");

        Orchestrator {
            shared,
            thread: Some(thread),
        }
    }

    /// Suspend between passes without invalidating the accumulated image.
    pub fn set_paused(&self, paused: bool) {
        if self.shared.paused.swap(paused, Ordering::AcqRel) != paused {
            self.shared.wake();
        }
    }

    /// Hand the render thread a new request.
    pub fn submit(&self, request: RenderRequest) {
        let mut slot = self.shared.request.lock().unwrap();
        if slot.as_ref() == Some(&request) {
            return;
        }
        *slot = Some(request);
        self.shared
            .request_generation
            .fetch_add(1, Ordering::Release);
        drop(slot);
        self.shared.wake();
    }

    /// Change the image style.
    ///
    /// Deliberately separate from [`Self::submit`]: `docs/01` §9.3 classifies
    /// an image-style change as needing no re-render at all, and routing it
    /// through the request would discard every accumulated sample for a
    /// slider drag.
    pub fn set_style(&self, style: ImageStyle) {
        let mut slot = self.shared.style.lock().unwrap();
        if *slot == style {
            return;
        }
        *slot = style;
        self.shared.style_generation.fetch_add(1, Ordering::Release);
        drop(slot);
        self.shared.wake();
    }

    /// Change the display pipeline. Free for the same reason as the style.
    ///
    /// Compared by label rather than by content: a `Processor` holds LUT
    /// tables, and comparing two of them element by element on every frame
    /// would cost more than the re-develop it is trying to avoid. The label is
    /// the selection, and the selection is what changes.
    pub fn set_display(&self, display: DisplayPipeline) {
        let mut slot = self.shared.display.lock().unwrap();
        if slot.label() == display.label() {
            return;
        }
        *slot = display;
        self.shared.style_generation.fetch_add(1, Ordering::Release);
        drop(slot);
        self.shared.wake();
    }

    /// Which instances to outline. Free, for the same reason the style is.
    pub fn set_selection(&self, mut selected: Vec<u32>) {
        selected.sort_unstable();
        let mut slot = self.shared.selection.lock().unwrap();
        if *slot == selected {
            return;
        }
        *slot = selected;
        self.shared.style_generation.fetch_add(1, Ordering::Release);
        drop(slot);
        self.shared.wake();
    }

    /// What a ray through the viewport hits, for selection.
    ///
    /// `None` before the first scene is built, and while the render thread is
    /// mid-rebuild — a click in that window selects nothing, which is right:
    /// there is nothing on screen to have clicked.
    pub fn pick(&self, ray: &Ray) -> Option<Hit> {
        let intersector = self.shared.intersector.lock().unwrap().clone()?;
        if self.shared.request.lock().unwrap().as_ref().is_some_and(|r| r.shading == Shading::Fast) {
            return tequila_render_core::preview::intersect_solid(intersector.as_ref(), *ray);
        }
        intersector.intersect(ray)
    }

    /// What the viewport has to say about its engine, if anything.
    pub fn engine_note(&self) -> Option<EngineNote> {
        self.shared.engine_note.lock().unwrap().clone()
    }

    /// The most recent finished image, if it has changed since `seen`.
    pub fn take_image(&self, seen: &mut u64) -> Option<DisplayImage> {
        let generation = self.shared.image_generation.load(Ordering::Acquire);
        if generation == *seen {
            return None;
        }
        *seen = generation;
        Some(self.shared.image.lock().unwrap().clone())
    }

    /// What the render thread was last asked for.
    #[cfg(test)]
    pub fn request(&self) -> Option<RenderRequest> {
        self.shared.request.lock().unwrap().clone()
    }
}

/// Set when the render thread had not stopped as the application quit
/// (`Drop for Orchestrator`), for `main` to end the process at once.
static STUCK_ON_EXIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a render thread was left running as the application quit.
pub fn stuck_on_exit() -> bool {
    STUCK_ON_EXIT.load(Ordering::Acquire)
}

impl Drop for Orchestrator {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::Release);
        self.shared.wake();
        if let Some(thread) = self.thread.take() {
            // Cycles making its kernels for a graphics card - minutes, the
            // first time on a Mac - holds its scene while it does, and the
            // render thread waits for the scene: quitting waited as long,
            // and a window that would not close was force-quit, which reads
            // as a crash. A few seconds, then the application leaves it.
            let deadline = Instant::now() + Duration::from_secs(5);
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if thread.is_finished() {
                let _ = thread.join();
            } else {
                STUCK_ON_EXIT.store(true, Ordering::Release);
            }
        }
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}

/// Everything the render loop rebuilds only when it has to.
struct Built {
    request: RenderRequest,
    intersector: Arc<EmbreeIntersector>,
    framebuffer: Framebuffer,
    schedule: ProgressiveSchedule,
    context: RenderContext,
    started: Instant,
    first_pixel_ms: Option<f64>,
}

fn render_loop(shared: Arc<Shared>) {
    // Pay the one-off table build before the first frame rather than inside
    // it: §9.2 budgets 100 ms to first pixels, and this costs about half that.
    tequila_materials::energy::prewarm();

    let device = match Device::shared() {
        Ok(device) => device,
        Err(error) => {
            eprintln!("Tequila: {error}");
            return;
        }
    };
    let denoiser = Denoiser::shared().ok();
    let cpu_denoiser: Option<&dyn tequila_denoise::Denoise> = denoiser.as_deref().map(|d| d as &dyn tequila_denoise::Denoise);
    // The GPU denoiser, for frames Cycles made on the graphics card: made
    // the first time one is wanted, and only then.
    let mut gpu_denoiser: Option<Arc<tequila_denoise::GpuDenoiser>> = None;

    let mut built: Option<Built> = None;
    let mut seen_request = 0u64;
    let mut seen_style = 0u64;
    let mut style = ImageStyle::default();
    let mut display = DisplayPipeline::default();
    let mut selection: Vec<u32> = Vec::new();
    let mut paused_at: Option<Instant> = None;
    // The Cycles view, kept across requests: making one loads the GPU
    // kernels, which takes a second or two. An engine that would not start is
    // remembered, so the viewport falls back to Tequila's own renderer rather
    // than trying again every frame.
    let mut cycles: Option<crate::cycles_view::CyclesView> = None;
    let mut cycles_failed: Option<tequila_scene::render::RenderEngine> = None;
    let mut cycles_status = String::new();
    // Why the engine that failed did, and how many frames the Cycles view
    // has shown since it was made: none yet means it is still getting ready.
    let mut cycles_why = String::new();
    let mut cycles_frames = 0u64;
    // Tequila's floor reflection under Cycles, refined a sample a frame
    // alongside it, and what it is of: the scene, the camera and the size of
    // the frames it goes over.
    let mut reflection: Option<(Framebuffer, (usize, Camera, usize, usize))> = None;
    let say = |note: Option<EngineNote>| {
        let mut current = shared.engine_note.lock().unwrap();
        if *current != note {
            *current = note;
        }
    };
    // When the Cycles view was last denoised, how many samples its last frame
    // had (fewer means the view restarted), and whether the picture showing
    // is a denoised one of the current view.
    let mut cycles_denoised_at = Instant::now();
    let mut cycles_wait = Duration::ZERO;
    let mut cycles_clean = false;
    // When a display change last put up an undenoised picture over a
    // denoised Cycles one: denoised again once the changes stop, since a
    // view that has finished sends no new frame that would be.
    let mut restyled: Option<Instant> = None;

    while shared.running.load(Ordering::Acquire) {
        let request_generation = shared.request_generation.load(Ordering::Acquire);
        let style_generation = shared.style_generation.load(Ordering::Acquire);

        if style_generation != seen_style {
            seen_style = style_generation;
            style = *shared.style.lock().unwrap();
            display = shared.display.lock().unwrap().clone();
            selection.clear();
            selection.extend_from_slice(&shared.selection.lock().unwrap());
            // Re-tonemap the existing accumulation. No samples are discarded:
            // this is the `Dirty::Display` row of §9.3, and it is what makes
            // dragging an exposure slider instant regardless of scene size.
            if let Some(built) = built.as_mut() {
                publish(
                    &shared,
                    built,
                    style,
                    &display,
                    &selection,
                    cpu_denoiser,
                    false,
                );
            }
            if cycles.is_some() && cycles_clean {
                restyled = Some(Instant::now());
            }
        }

        // Keep display edits available while paused, but defer scene builds
        // and tracing. Resuming consumes the latest request only.
        if shared.paused.load(Ordering::Acquire) {
            if let Some(view) = cycles.as_mut() {
                view.set_paused(true);
            }
            paused_at.get_or_insert_with(Instant::now);
            shared.parked.store(true, Ordering::Release);
            let guard = shared.wakeup_lock.lock().unwrap();
            let _ = shared.wakeup.wait_timeout(guard, Duration::from_millis(100)).unwrap();
            continue;
        }
        shared.parked.store(false, Ordering::Release);
        if let Some(start) = paused_at.take() {
            if let Some(state) = built.as_mut() {
                state.started += start.elapsed();
            }
        }

        if request_generation != seen_request {
            seen_request = request_generation;
            let Some(request) = shared.request.lock().unwrap().clone() else {
                continue;
            };
            // Release the UI's reference first. An in-place transform
            // update needs `&mut` on the intersector, which means being its
            // only owner; holding the picking clone across the rebuild would
            // silently force a full BVH rebuild on every drag frame.
            //
            // The window where picking finds nothing is the window in which
            // there is nothing on screen to pick anyway.
            shared.intersector.lock().unwrap().take();

            let state = crate::profile::time("render: rebuild", || rebuild(built.take(), request, &device));

            // Publish the BVH the UI picks against, so a click can only ever
            // hit geometry that is actually on screen.
            *shared.intersector.lock().unwrap() = Some(state.intersector.clone());
            built = Some(state);
        }

        let Some(state) = built.as_mut() else {
            // Nothing to render yet. Wait rather than spin.
            wait_for_news(&shared, seen_request, seen_style, Duration::from_millis(50));
            continue;
        };

        // A Rendered viewport on the Cycles engine shows Cycles' newest frame.
        let engine = state.request.engine;
        let on_cycles = state.request.shading == Shading::Rendered && engine.is_cycles();
        if on_cycles && cycles_failed == Some(engine) {
            say(Some(EngineNote::FellBack { engine, why: cycles_why.clone() }));
        } else if !on_cycles {
            say(None);
        }
        if on_cycles && cycles_failed != Some(engine) {
            if cycles.as_ref().map(|view| view.engine()) != Some(engine) {
                cycles = None;
                cycles_frames = 0;
                match crate::profile::time("render: start cycles", || crate::cycles_view::CyclesView::new(engine)) {
                    Ok(view) => cycles = Some(view),
                    Err(error) => {
                        eprintln!("Tequila: Cycles could not start, so the viewport uses Tequila's renderer: {error}");
                        cycles_failed = Some(engine);
                        cycles_why = error;
                        continue;
                    }
                }
            }
            let view = cycles.as_mut().expect("made above");
            view.set_paused(false);
            match crate::profile::time("render: sync cycles", || view.sync(&state.request)) {
                Ok(true) => cycles_clean = false,
                Ok(false) => {}
                Err(error) => {
                    eprintln!("Tequila: Cycles could not show this scene, so the viewport uses Tequila's renderer: {error}");
                    cycles_failed = Some(engine);
                    cycles_why = error;
                    cycles = None;
                    continue;
                }
            }
            // A display change put up the picture undenoised, to answer at
            // once; denoised again when the changes pause.
            if let Some(at) = restyled {
                const SETTLE: Duration = Duration::from_millis(150);
                if at.elapsed() < SETTLE {
                    wait_for_news(&shared, seen_request, seen_style, SETTLE - at.elapsed());
                    continue;
                }
                restyled = None;
                if cycles_clean && state.request.denoise {
                    if engine == tequila_scene::render::RenderEngine::CyclesGpu && gpu_denoiser.is_none() {
                        gpu_denoiser = tequila_denoise::GpuDenoiser::shared().ok();
                    }
                    let view_denoiser = match (&gpu_denoiser, engine) {
                        (Some(gpu), tequila_scene::render::RenderEngine::CyclesGpu) => Some(gpu.as_ref() as &dyn tequila_denoise::Denoise),
                        _ => cpu_denoiser,
                    };
                    publish(&shared, state, style, &display, &selection, view_denoiser, true);
                    cycles_denoised_at = Instant::now();
                }
            }
            // Denoised frames are paced (see below). Until the next is due,
            // the frame is left with Cycles rather than taken and thrown
            // away: it only ever holds the newest, so the one taken when the
            // time comes - the last one, once the render is done - is current.
            if cycles_clean && state.request.denoise {
                let since = cycles_denoised_at.elapsed();
                if since < cycles_wait {
                    wait_for_news(&shared, seen_request, seen_style, (cycles_wait - since).min(Duration::from_millis(50)));
                    continue;
                }
            }
            // Cycles stopped on an error. A graphics card it could open but
            // not render on - older than the kernels shipped, or with a
            // driver too old to compile them - ends up here, after the start
            // and the scene both went through; waiting for frames that never
            // come left the viewport black, saying nothing. It falls back to
            // Tequila's renderer, as a refusal at the start does, and says
            // why. Its progress, until it has shown something, is the HUD's.
            match view.status() {
                Err(error) => {
                    if error != cycles_status {
                        eprintln!("Tequila: Cycles stopped, so the viewport uses Tequila's renderer: {error}");
                        cycles_status = error.clone();
                    }
                    cycles_failed = Some(engine);
                    cycles_why = error;
                    cycles = None;
                    continue;
                }
                Ok(doing) if cycles_frames == 0 => say(Some(EngineNote::Preparing(doing))),
                Ok(_) => {}
            }
            match view.take_frame(&mut state.framebuffer) {
                Some(frame) => {
                    if cycles_frames == 0 {
                        say(None);
                    }
                    cycles_frames += 1;
                    // The floor's reflection, Tequila's, laid over: Cycles
                    // draws none (`tequila_render_cpu::floor`).
                    if view.floor_reflection() {
                        let key = (Arc::as_ptr(&state.request.scene) as usize, state.request.camera, frame.width, frame.height);
                        if reflection.as_ref().is_none_or(|(_, of)| *of != key) {
                            reflection = Some((Framebuffer::new(frame.width, frame.height), key));
                        }
                        let (layer, _) = reflection.as_mut().expect("made above");
                        if layer.pixels().first().map_or(0, |p| p.samples) < 256 {
                            tequila_render_cpu::floor::render_floor_reflection(
                                state.intersector.as_ref(),
                                layer,
                                state.request.camera,
                                &state.request.config,
                                1,
                            );
                        }
                        for (pixel, laid) in state.framebuffer.pixels_mut().iter_mut().zip(layer.pixels()) {
                            let share = laid.alpha();
                            if share > 0.0 && pixel.samples > 0 {
                                let n = pixel.samples as f32;
                                let (colour, coverage) = tequila_render_cpu::floor::lay_reflection(
                                    pixel.sum * (1.0 / n),
                                    pixel.coverage_sum / n,
                                    laid.mean(),
                                    share,
                                );
                                pixel.sum = colour * n;
                                pixel.coverage_sum = coverage * n;
                            }
                        }
                    } else {
                        reflection = None;
                    }
                    // Denoised here, with the frame's own guides, as the
                    // document asks - but not every frame: at full size the
                    // denoiser costs more than the samples between two frames,
                    // and denoising each one held the view a second behind
                    // the camera. Early on, the quick filter often; once the
                    // image settles, the careful one now and then. In between,
                    // the denoised picture already showing stays, rather than
                    // flickering to a noisy one.
                    let early = frame.samples < 32 || state.request.interacting;
                    // The first frames after a change go up as they are: a
                    // view that answers the mouse at once matters more than a
                    // smooth one a moment later.
                    let denoise = state.request.denoise && frame.samples >= 8 && !state.request.interacting;
                    cycles_wait = Duration::from_millis(if early { 150 } else { 1000 });
                    if early {
                        state.schedule.interrupt();
                    } else {
                        state.schedule.update(60_000);
                    }
                    state.schedule.resolution_divisor = (state.request.width / frame.width.max(1)).max(1) as u32;
                    if denoise && engine == tequila_scene::render::RenderEngine::CyclesGpu && gpu_denoiser.is_none() {
                        gpu_denoiser = tequila_denoise::GpuDenoiser::shared().ok();
                    }
                    let view_denoiser = match (&gpu_denoiser, engine) {
                        (Some(gpu), tequila_scene::render::RenderEngine::CyclesGpu) => Some(gpu.as_ref() as &dyn tequila_denoise::Denoise),
                        _ => cpu_denoiser,
                    };
                    publish(&shared, state, style, &display, &selection, view_denoiser, denoise);
                    if denoise {
                        cycles_denoised_at = Instant::now();
                        cycles_clean = true;
                    }
                }
                None => wait_for_news(&shared, seen_request, seen_style, Duration::from_millis(15)),
            }
            continue;
        }
        if let Some(view) = cycles.as_mut() {
            // Not showing, so not using the graphics card either.
            view.set_paused(true);
        }

        // Interactive frames render at reduced resolution with one sample; a
        // refining pass accumulates several at a time so the per-pass overhead
        // does not dominate once the image is nearly converged (§9.2).
        let samples = if state.schedule.is_interactive() {
            1
        } else {
            4
        };

        if state.request.shading == Shading::Fast && state.request.gpu_solid {
            // The picture is the GPU's. The BVH above is still wanted, for
            // clicking on parts; tracing a frame nobody shows is not.
            wait_for_news(&shared, seen_request, seen_style, Duration::from_millis(100));
            continue;
        }
        if state.request.shading == Shading::Fast {
            // §9.2's resolution ladder does not apply here and using it was a
            // bug worth naming: the ladder climbs over about 300 ms of idle
            // time, a fast preview reaches its sample cap in about 200 ms, and
            // the loop then went to sleep at quarter resolution and stayed
            // there. The viewport showed a permanently blurry image and the
            // HUD said so, in a mode whose entire pitch is that it is instant.
            //
            // A fast frame is complete the moment it is drawn, so there is
            // nothing to climb: full resolution as soon as the pointer stops,
            // and half while it is moving, which is the only place the frame
            // rate is felt.
            let (width, height) = state.request.traced_size();
            let divisor = solid_divisor(width, height, state.request.interacting);
            if state.schedule.resolution_divisor != divisor {
                state.schedule.resolution_divisor = divisor;
                let (w, h) = state.schedule.render_resolution(width, height);
                state.framebuffer.resize(w, h);
                state.framebuffer.clear();
            }

            let cap = if state.request.interacting { 1 } else { Shading::FAST_SAMPLE_CAP };
            let completed = state.framebuffer.min_samples();
            if completed >= cap {
                // Finished, and nothing will change it until a request does.
                // Sleeping here is the point of the mode.
                wait_for_news(&shared, seen_request, seen_style, Duration::from_millis(100));
                continue;
            }
            crate::profile::time("render: solid trace", || preview_pass(
                state.intersector.as_ref(),
                &mut state.framebuffer,
                &state.context,
                if completed == 0 { 1 } else { cap - completed },
            ));
            // Shown even when the camera has already moved on. Throwing a
            // finished frame away for being one mouse-move old looked like
            // "only show what is current" and was the opposite: a mouse moves
            // the camera ~120 times a second and a frame takes tens of
            // milliseconds, so *every* frame was superseded before it was
            // done, and a drag showed nothing at all until the mouse stopped
            // (`solid_orbit_benchmark`: 0 images in 3 s). A frame one move
            // behind is the most current picture there is.
            // Never denoised: there is no noise, and a denoiser run over a
            // clean image only softens the edges the extra samples just built.
            let _publish = Timer::start("render: solid publish");
            publish(
                &shared,
                state,
                style,
                &display,
                &selection,
                cpu_denoiser,
                false,
            );
            continue;
        } else {
            render_pass(
                state.intersector.as_ref(),
                &mut state.framebuffer,
                &state.context,
                &state.schedule,
                samples,
            );

            let denoise = state.schedule.should_denoise_every(
                state.framebuffer.min_samples(),
                state.request.denoise_cadence,
            );
            publish(
                &shared,
                state,
                style,
                &display,
                &selection,
                cpu_denoiser,
                denoise,
            );
        }

        // Advance the progressive schedule. While the user is interacting the
        // schedule is pinned to the fast path; once they stop, it climbs the
        // resolution ladder.
        if state.request.interacting {
            state.schedule.interrupt();
        } else {
            let idle = state.started.elapsed().as_millis() as u64;
            let previous = state.schedule.resolution_divisor;
            state.schedule.update(idle);
            if state.schedule.resolution_divisor != previous {
                // A resolution step restarts accumulation, which §9.2 notes is
                // cheap: only a few samples are lost.
                let (width, height) = state.request.traced_size();
                let (w, h) = state.schedule.render_resolution(width, height);
                state.framebuffer.resize(w, h);
                state.framebuffer.clear();
            }
        }
    }
}

impl Shared {
    /// Wake the render loop. The lock is taken first so the wake-up cannot
    /// fall between the loop's check and its wait; see [`wait_for_news`].
    fn wake(&self) {
        let _guard = self.wakeup_lock.lock().unwrap();
        self.wakeup.notify_all();
    }
}

/// Sleep until a request or a style change arrives, or `timeout` passes.
///
/// The check is made under the lock every notifier takes before notifying,
/// which is what makes it safe. Checking first and then waiting — as this
/// used to — leaves a gap: a request landing between the check and the wait
/// sends its wake-up to nobody, and the renderer sleeps the full timeout with
/// work queued.
fn wait_for_news(shared: &Shared, seen_request: u64, seen_style: u64, timeout: Duration) {
    let guard = shared.wakeup_lock.lock().unwrap();
    let news = shared.request_generation.load(Ordering::Acquire) != seen_request
        || shared.style_generation.load(Ordering::Acquire) != seen_style
        || !shared.running.load(Ordering::Acquire);
    if !news {
        let _ = shared.wakeup.wait_timeout(guard, timeout).unwrap();
    }
}

/// Rebuild whatever the new request invalidated.
fn solid_divisor(width: usize, height: usize, interacting: bool) -> u32 {
    if !interacting { return 1; }
    let mut divisor = 2u32;
    while (width / divisor as usize) * (height / divisor as usize) > 300_000 { divisor += 1; }
    divisor
}

/// Records the time until it is dropped.
struct Timer(&'static str, Instant);

impl Timer {
    fn start(stage: &'static str) -> Timer {
        Timer(stage, Instant::now())
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        crate::profile::record(self.0, self.1.elapsed().as_secs_f64() * 1000.0);
    }
}

fn rebuild(mut previous: Option<Built>, request: RenderRequest, device: &Arc<Device>) -> Built {
    let mut framebuffer = previous.as_mut().map(|p| std::mem::replace(&mut p.framebuffer, Framebuffer::new(0, 0)));
    let scene_changed = previous
        .as_ref()
        .is_none_or(|p| !Arc::ptr_eq(&p.request.scene, &request.scene));

    let intersector = match previous {
        Some(p) if !scene_changed => p.intersector,
        Some(p) => {
            // `docs/01` §9.3: an instance transform change rebuilds the TLAS and
            // nothing else. A gizmo drag issues one per frame, and rebuilding
            // every per-mesh BLAS each time is what makes dragging a large
            // assembly unusable. `update_transforms` refuses anything that is
            // not purely a re-placement, so the fallback is still correct for
            // every other kind of edit.
            let mut intersector = p.intersector;
            let updated =
                Arc::get_mut(&mut intersector).is_some_and(|i| i.update_transforms(&request.scene));
            if updated {
                intersector
            } else {
                Arc::new(
                    EmbreeIntersector::build(device, (*request.scene).clone())
                        .expect("failed to build the scene BVH"),
                )
            }
        }
        None => Arc::new(
            EmbreeIntersector::build(device, (*request.scene).clone())
                .expect("failed to build the scene BVH"),
        ),
    };

    let mut schedule = ProgressiveSchedule::default();
    if !request.interacting {
        schedule.update(0);
    }
    let (width, height) = request.traced_size();
    if request.shading == Shading::Fast {
        schedule.resolution_divisor = solid_divisor(width, height, request.interacting);
    }
    let (w, h) = schedule.render_resolution(width, height);
    let mut framebuffer = framebuffer.take().unwrap_or_else(|| Framebuffer::new(w, h));
    framebuffer.resize(w, h);
    framebuffer.clear();

    Built {
        context: RenderContext {
            camera: request.camera,
            config: request.config.clone(),
            scene_seed: 0,
            reject_fireflies: request.reject_fireflies,
            clamp_sample: request.clamp_sample,
            region: None,
            ..Default::default()
        },
        framebuffer,
        schedule,
        started: Instant::now(),
        first_pixel_ms: None,
        request,
        intersector,
    }
}

/// Ring the selected instances in the developed image.
///
/// Drawn from the per-pixel instance id rather than from projected geometry,
/// which is what makes it a true silhouette: it follows the shape, it is
/// correct through a hole in a bracket, and a part hidden behind another is
/// not outlined because none of its pixels are on screen. A projected bounding
/// box would be cheaper and would answer a different question.
///
/// Two pixels wide and drawn *outside* the selection — on the background side
/// of the boundary rather than over the part — so selecting something never
/// hides the edge of the thing being selected. That is also why it goes on
/// after the develop: it is an annotation, not part of the picture, and must
/// not be graded, tone-mapped or denoised with it. `density` is the image's
/// pixels per point, one or more.
fn outline_selection(
    pixels: &mut [u8],
    fb: &Framebuffer,
    width: usize,
    height: usize,
    density: f32,
    selected: &[u32],
) {
    if selected.is_empty() || width == 0 || height == 0 {
        return;
    }
    // Which pixels are the selection, asked once each — and the box they
    // fill, since a ring can only be near them. Asking every neighbour of
    // every pixel instead (a search through the selection each time) cost
    // 35 ms per published frame at viewport size.
    let mut mask = vec![false; width * height];
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..height {
        for x in 0..width {
            let id = fb.pixel(x, y).first_instance;
            if id != u32::MAX && selected.binary_search(&id).is_ok() {
                mask[y * width + x] = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 == usize::MAX {
        return;
    }

    // The accent: no material is that saturated a teal, so the ring never
    // reads as part of the object it is around.
    let accent = crate::theme::p().accent;
    let ring = [accent.r(), accent.g(), accent.b()];
    // Measured in points: a Retina frame ringed two of its own pixels wide
    // had half the ring Solid draws around the same part.
    let points = (width as f32 / density).min(height as f32 / density);
    let radius = ((if points > 700.0 { 2.0 } else { 1.0 }) * density).round() as usize;

    // A background pixel within `radius` of a selected one, both ways: the
    // square neighbourhood is a row pass and then a column pass.
    let (bx0, bx1) = (x0.saturating_sub(radius), (x1 + radius).min(width - 1));
    let (by0, by1) = (y0.saturating_sub(radius), (y1 + radius).min(height - 1));
    let mut across = vec![false; width * height];
    for y in y0..=y1 {
        for x in bx0..=bx1 {
            let (lo, hi) = (x.saturating_sub(radius), (x + radius).min(width - 1));
            across[y * width + x] = mask[y * width + lo..=y * width + hi].iter().any(|m| *m);
        }
    }
    for y in by0..=by1 {
        let (lo, hi) = (y.saturating_sub(radius), (y + radius).min(height - 1));
        for x in bx0..=bx1 {
            if mask[y * width + x] {
                continue;
            }
            if (lo..=hi).any(|k| across[k * width + x]) {
                let i = (y * width + x) * 4;
                pixels[i] = ring[0];
                pixels[i + 1] = ring[1];
                pixels[i + 2] = ring[2];
            }
        }
    }
}

/// Tone-map the accumulation buffer and hand it to the UI.
fn publish(
    shared: &Arc<Shared>,
    state: &mut Built,
    style: ImageStyle,
    display: &DisplayPipeline,
    selection: &[u32],
    denoiser: Option<&dyn tequila_denoise::Denoise>,
    denoise: bool,
) {
    let fb = &state.framebuffer;
    let (width, height) = (fb.width, fb.height);
    if width == 0 || height == 0 {
        return;
    }

    // Denoise for display only. §13: "Denoising is applied for display and for
    // output, never into the accumulation buffer." Accumulating a denoised
    // result would poison convergence, and the API here makes that impossible
    // — the denoised pixels never travel back.
    let denoised = crate::profile::time("publish: denoise", || denoise.then_some(denoiser).flatten().and_then(|denoiser| {
        let buffers = fb.to_denoiser_buffers();
        denoiser
            .denoise(
                width,
                height,
                &buffers.color,
                &buffers.albedo,
                &buffers.normal,
                DenoiseSettings {
                    quality: if state.schedule.is_interactive() {
                        Quality::Fast
                    } else {
                        Quality::Balanced
                    },
                    strength: state.request.denoise_strength,
                    // Automatic unless the user said otherwise: guides are
                    // themselves noisy early on and clean later, so the sample
                    // count is the right judge right up until someone
                    // disagrees with it.
                    prefilter_guides: state
                        .request
                        .prefilter_guides
                        .unwrap_or(fb.min_samples() < 32),
                },
            )
            .ok()
    }));

    // Gathered first, then developed as one image. The photographic style's
    // bloom, sharpening, vignette and chromatic aberration are defined over a
    // neighbourhood, and a per-pixel loop cannot express any of them however it
    // is called — a viewport that developed pixel by pixel would silently drop
    // four controls the Image tab offers.
    //
    // A row per task: this walks every pixel of the accumulation buffer, which
    // is about 80 bytes each, and done on one core it was most of the cost
    // of showing a Solid frame — 8 of every 11 ms.
    use rayon::prelude::*;
    let source = fb.pixels();
    let mut scene_linear = vec![tequila_core::spectrum::Spectrum::ZERO; width * height];
    scene_linear.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let color = match &denoised {
                Some(image) => {
                    let i = (y * width + x) * 3;
                    tequila_core::spectrum::Spectrum::from_rendering_rgb(Vec3 {
                        x: image.data[i],
                        y: image.data[i + 1],
                        z: image.data[i + 2],
                    })
                }
                None => source[y * width + x].mean(),
            };
            *out = color.sanitized();
        }
    });
    // A fast frame is display-referred already and carries no physical
    // radiance, so it is published through a plain sRGB transform with the
    // grade left at neutral. Running the user's exposure and their filmic
    // curve over it would be applying a photographic look to something that is
    // not a photograph — which is what turned the first version of this mode
    // into flat grey mush.
    let (style, display) = match state.request.shading {
        Shading::Fast => (
            ImageStyle::default(),
            &DisplayPipeline::Builtin(tequila_color::DisplayTransform::Srgb),
        ),
        Shading::Rendered => (style, display),
    };
    let mut pixels = crate::profile::time("publish: develop", || style.develop_through(width, height, &scene_linear, display));
    // The image's own pixels per point: a Retina screen's for a full-size
    // Cycles frame, and one for Tequila's, which traces the points.
    let density = (width as f32 * state.request.pixels_per_point / state.request.width.max(1) as f32).max(1.0);
    crate::profile::time("publish: outline", || outline_selection(&mut pixels, fb, width, height, density, selection));

    let elapsed = state.started.elapsed().as_secs_f64();
    if state.first_pixel_ms.is_none() {
        state.first_pixel_ms = Some(elapsed * 1000.0);
    }
    let samples = fb.min_samples();

    let mut slot = shared.image.lock().unwrap();
    *slot = DisplayImage {
        width,
        height,
        pixels,
        samples,
        // Another walk over every pixel, and a question only a progressive
        // render asks.
        converged_fraction: match state.request.shading {
            Shading::Fast => 1.0,
            Shading::Rendered => fb.converged_fraction(),
        },
        samples_per_second: if elapsed > 0.0 {
            (samples as f64 * (width * height) as f64) / elapsed
        } else {
            0.0
        },
        triangles: state.intersector.scene().triangle_count(),
        instances: state.intersector.scene().instances.len(),
        milliseconds_to_first_pixel: state.first_pixel_ms.unwrap_or(0.0),
        denoised: denoised.is_some(),
        elapsed_seconds: elapsed,
        resolution_divisor: state.schedule.resolution_divisor,
        shading: state.request.shading,
    };
    drop(slot);
    shared.image_generation.fetch_add(1, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_preserves_samples_and_defers_new_requests() {
        fn until(mut ready: impl FnMut() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !ready() {
                assert!(Instant::now() < deadline, "renderer did not reach the expected state");
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let renderer = Orchestrator::new();
        let mut req = request();
        req.width = 32;
        req.height = 24;
        req.denoise = false;
        req.config.adaptive_sampling_threshold = 0.0;
        renderer.submit(req.clone());
        until(|| {
            let image = renderer.shared.image.lock().unwrap();
            image.resolution_divisor == 1 && image.samples >= 8
        });
        renderer.set_paused(true);
        until(|| renderer.shared.parked.load(Ordering::Acquire));
        let before = renderer.shared.image.lock().unwrap().clone();
        let generation = renderer.shared.image_generation.load(Ordering::Acquire);
        std::thread::sleep(Duration::from_millis(220));
        assert_eq!(generation, renderer.shared.image_generation.load(Ordering::Acquire));

        // Exposure still updates the displayed image without taking a sample.
        let mut style = ImageStyle::default();
        style.exposure = 1.0;
        renderer.set_style(style);
        until(|| renderer.shared.image_generation.load(Ordering::Acquire) > generation);
        assert_eq!(before.samples, renderer.shared.image.lock().unwrap().samples);
        renderer.set_paused(false);
        until(|| renderer.shared.image.lock().unwrap().samples > before.samples);

        renderer.set_paused(true);
        until(|| renderer.shared.parked.load(Ordering::Acquire));
        req.width = 48;
        renderer.submit(req);
        std::thread::sleep(Duration::from_millis(220));
        assert_eq!(renderer.shared.image.lock().unwrap().width, 32);
        renderer.set_paused(false);
        until(|| renderer.shared.image.lock().unwrap().width == 48);
    }

    #[test]
    fn solid_starts_by_default_and_bounds_interactive_work() {
        assert_eq!(Shading::default(), Shading::Fast);
        for (w, h) in [(1920, 1080), (3840, 2160), (7680, 4320)] {
            let divisor = solid_divisor(w, h, true) as usize;
            assert!((w / divisor) * (h / divisor) <= 300_000);
            assert_eq!(solid_divisor(w, h, false), 1);
        }
    }

    #[test]
    fn camera_updates_reuse_the_accumulation_storage() {
        let device = Device::shared().unwrap();
        let mut req = request();
        req.shading = Shading::Fast;
        let first = rebuild(None, req.clone(), &device);
        let storage = first.framebuffer.pixels().as_ptr();
        req.camera = Camera::look_at(Vec3::splat(5.0), Vec3::ZERO, Vec3::Y);
        let second = rebuild(Some(first), req, &device);
        assert_eq!(storage, second.framebuffer.pixels().as_ptr());
        assert_eq!(second.framebuffer.min_samples(), 0);
    }

    /// What the Solid viewport actually shows while the camera is being
    /// dragged: a mouse moves the camera ~120 times a second, and this counts
    /// how many images come out the other end.
    ///
    /// Ignored by default (it takes a few seconds and wants a real scene):
    ///
    /// ```text
    /// TEQUILA_BENCH_SCENE=path/to/scene.tequila \
    ///   cargo test --release -p tequila-app solid_orbit_benchmark -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn solid_orbit_benchmark() {
        let (scene, center, radius) = match std::env::var("TEQUILA_BENCH_SCENE") {
            Ok(path) => {
                let loaded = tequila_format::load(std::path::Path::new(&path)).expect("the scene loads");
                let bounds = tequila_scene::snapshot::visible_bounds(&loaded.document);
                let project = crate::project::Project::with_document(
                    loaded.document,
                    Arc::new(tequila_scene::demo::studio_environment()),
                );
                let center = (bounds.min + bounds.max) * 0.5;
                (project.scene(), center, (bounds.max - bounds.min).length() * 1.2)
            }
            Err(_) => (request().scene, Vec3::ZERO, 6.0),
        };
        let (width, height) = (1400, 800);
        let camera_at = |angle: f32| {
            let eye = center + Vec3 { x: angle.cos() * radius, y: radius * 0.4, z: angle.sin() * radius };
            Camera::look_at(eye, center, Vec3::Y)
        };
        let mut req = request();
        req.scene = scene;
        req.width = width;
        req.height = height;
        req.shading = Shading::Fast;
        req.interacting = true;

        let orchestrator = Orchestrator::new();
        req.camera = camera_at(0.0);
        orchestrator.submit(req.clone());
        let mut seen = 0u64;
        let warm = Instant::now();
        while orchestrator.take_image(&mut seen).is_none() {
            assert!(warm.elapsed() < Duration::from_secs(60), "no first image");
            std::thread::sleep(Duration::from_millis(5));
        }
        println!("first image after {:.0} ms (BVH build included)", warm.elapsed().as_secs_f64() * 1000.0);

        // Drag: a new camera every 1/120 s for three seconds.
        let started = Instant::now();
        let mut shown = 0;
        let mut frames = 0u32;
        while started.elapsed() < Duration::from_secs(3) {
            frames += 1;
            req.camera = camera_at(frames as f32 * 0.01);
            orchestrator.submit(req.clone());
            let tick = Instant::now();
            while tick.elapsed() < Duration::from_micros(8_333) {
                if orchestrator.take_image(&mut seen).is_some() {
                    shown += 1;
                }
                std::thread::sleep(Duration::from_micros(500));
            }
        }
        let seconds = started.elapsed().as_secs_f64();
        println!(
            "while dragging: {frames} camera moves, {shown} images shown = {:.1} images/s",
            shown as f64 / seconds
        );
    }

    /// What a Retina viewport costs: the same view at a pixel per point and
    /// at a Retina screen's pixels (an M3 Max MacBook's at the Compact text
    /// size, 889 x 908 points at 1.84 pixels a point), on Tequila's renderer
    /// and on Cycles on the graphics card - and Tequila's renderer tracing
    /// all the Retina pixels too, which is why it does not. For each: how
    /// long a camera move takes to come back denoised at full size, the
    /// samples a second after it, the pictures a second while orbiting and
    /// how wide they are, and how long letting go takes to come back clean.
    ///
    /// Ignored by default (it takes a minute or two). Cycles is measured when
    /// it is found; `TEQUILA_BENCH_SCENE` measures a scene of your own:
    ///
    /// ```text
    /// TEQUILA_CYCLES=path/to/cycles/build/bin \
    ///   cargo test --release -p tequila-app retina_viewport_benchmark -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn retina_viewport_benchmark() {
        use tequila_scene::render::RenderEngine;
        let document = match std::env::var("TEQUILA_BENCH_SCENE") {
            Ok(path) => tequila_format::load(std::path::Path::new(&path)).expect("the scene loads").document,
            Err(_) => crate::starter_document(),
        };
        let bounds = tequila_scene::snapshot::visible_bounds(&document);
        let project = crate::project::Project::with_document(document, Arc::new(tequila_scene::demo::studio_environment()));
        let center = (bounds.min + bounds.max) * 0.5;
        let radius = (bounds.max - bounds.min).length() * 1.2;
        let camera_at = |angle: f32| {
            let eye = center + Vec3 { x: angle.sin() * radius, y: radius * 0.4, z: angle.cos() * radius };
            Camera::look_at(eye, center, Vec3::Y)
        };
        // As the application asks, from the document's settings.
        let settings = project.render_settings().clone();
        let base = RenderRequest {
            camera: camera_at(0.0),
            width: 0,
            height: 0,
            config: project.integrator_config(),
            scene: project.scene(),
            scene_generation: project.generation(),
            denoise: settings.denoise,
            denoise_strength: settings.denoise_strength,
            denoise_cadence: settings.denoise_cadence,
            prefilter_guides: Some(settings.prefilter_guides),
            reject_fireflies: settings.reject_fireflies,
            clamp_sample: settings.clamp_indirect,
            interacting: false,
            gpu_solid: false,
            shading: Shading::Rendered,
            engine: RenderEngine::Tequila,
            pixels_per_point: 1.0,
        };
        let mut engines = vec![RenderEngine::Tequila];
        match tequila_render_cycles::available() {
            Ok(()) => engines.push(RenderEngine::CyclesGpu),
            Err(why) => println!("Cycles not measured: {why}"),
        }
        // The first picture at full size and denoised, and how long it took.
        let clean = |orchestrator: &Orchestrator, seen: &mut u64, width: usize, within: Duration| {
            let started = Instant::now();
            loop {
                if let Some(image) = orchestrator.take_image(seen) {
                    if image.width == width && image.denoised {
                        return Some((started.elapsed(), image));
                    }
                }
                if started.elapsed() > within {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        println!(
            "{:<10} {:>10} {:>10} {:>9} {:>8} {:>8} {:>10} {:>11} {:>10}",
            "engine", "asked", "traced", "clean ms", "spp/s", "Msamp/s", "orbit fps", "orbit width", "let go ms"
        );
        for engine in engines {
            let mut sizes = vec![(889, 908, 1.0), (1636, 1671, 1.84)];
            if engine == RenderEngine::Tequila {
                sizes.push((1636, 1671, 1.0));
            }
            for (width, height, pixels_per_point) in sizes {
                let orchestrator = Orchestrator::new();
                let mut seen = 0u64;
                let mut req = RenderRequest { width, height, engine, pixels_per_point, ..base.clone() };
                // What comes back: Tequila traces the points, Cycles the pixels.
                let (full, tall) = if engine == RenderEngine::Tequila { req.traced_size() } else { (width, height) };
                orchestrator.submit(req.clone());
                // Cycles starting, the denoiser made and the BVH built are
                // the same at either size: paid here, not measured.
                clean(&orchestrator, &mut seen, full, Duration::from_secs(600)).expect("no clean picture at first");

                req.camera = camera_at(0.6);
                orchestrator.submit(req.clone());
                let moved = Instant::now();
                let (first, mut latest) =
                    clean(&orchestrator, &mut seen, full, Duration::from_secs(60)).expect("no clean picture after a move");
                while moved.elapsed() < Duration::from_secs(5) {
                    if let Some(image) = orchestrator.take_image(&mut seen) {
                        latest = image;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                let spp = latest.samples as f64 / latest.elapsed_seconds.max(1e-3);

                // Orbiting: a new camera every 1/60 s for three seconds.
                req.interacting = true;
                let orbit = Instant::now();
                let (mut moves, mut widths) = (0u32, Vec::new());
                while orbit.elapsed() < Duration::from_secs(3) {
                    moves += 1;
                    req.camera = camera_at(0.6 + moves as f32 * 0.01);
                    orchestrator.submit(req.clone());
                    let tick = Instant::now();
                    while tick.elapsed() < Duration::from_micros(16_667) {
                        if let Some(image) = orchestrator.take_image(&mut seen) {
                            widths.push(image.width);
                        }
                        std::thread::sleep(Duration::from_micros(500));
                    }
                }
                let fps = widths.len() as f64 / orbit.elapsed().as_secs_f64();
                widths.sort_unstable();
                let typical = widths.get(widths.len() / 2).copied().unwrap_or(0);
                req.interacting = false;
                orchestrator.submit(req.clone());
                let (released, _) =
                    clean(&orchestrator, &mut seen, full, Duration::from_secs(60)).expect("no clean picture after letting go");
                println!(
                    "{:<10} {:>10} {:>10} {:>9.0} {:>8.1} {:>8.1} {:>10.1} {:>11} {:>10.0}",
                    format!("{engine:?}"),
                    format!("{width}x{height}"),
                    format!("{full}x{tall}"),
                    ms(first),
                    spp,
                    spp * (full * tall) as f64 / 1e6,
                    fps,
                    typical,
                    ms(released)
                );
            }
        }
    }

    /// The Solid pass runs each material's program pruned to what Solid
    /// shading reads; on a real textured scene, every pixel must come out
    /// exactly as it would from the full program.
    #[test]
    #[ignore]
    fn pruned_solid_programs_do_not_change_the_picture() {
        use tequila_render_core::preview;
        let path = std::env::var("TEQUILA_BENCH_SCENE").expect("set TEQUILA_BENCH_SCENE to a textured scene");
        let loaded = tequila_format::load(std::path::Path::new(&path)).expect("the scene loads");
        let bounds = tequila_scene::snapshot::visible_bounds(&loaded.document);
        let project = crate::project::Project::with_document(loaded.document, Arc::new(tequila_scene::demo::studio_environment()));
        let scene = project.scene();
        let textured = scene.programs.iter().flatten().filter(|p| !p.instructions.is_empty()).count();
        assert!(textured > 0, "the scene has no textured material to test with");
        let intersector = EmbreeIntersector::build(&Device::shared().unwrap(), (*scene).clone()).unwrap();
        let center = (bounds.min + bounds.max) * 0.5;
        let camera = Camera::look_at(center + Vec3 { x: 0.4, y: 0.3, z: 0.5 }, center, Vec3::Y);
        let rig = preview::Rig::for_view(camera.forward(), Vec3::Y);
        let pruned = preview::SolidPrograms::for_scene(&scene);
        let mut registers = tequila_render_core::shade::Registers::new();
        let mut scratch = tequila_render_core::material::Material::default();
        let (w, h) = (320, 200);
        let mut hits = 0;
        for y in 0..h {
            for x in 0..w {
                let film = tequila_core::math::Vec2 { x: (x as f32 + 0.5) / w as f32, y: (y as f32 + 0.5) / h as f32 };
                let ray = camera.pick_ray(film, w as f32 / h as f32);
                let full = preview::shade_ray(&intersector, ray, film, &rig, &mut registers, &mut scratch);
                let fast = preview::shade_ray_with(&intersector, ray, film, &rig, Some(&pruned), &mut registers, &mut scratch);
                assert_eq!(full.radiance, fast.radiance, "pixel ({x}, {y})");
                assert_eq!(full.albedo, fast.albedo, "pixel ({x}, {y})");
                hits += (!full.escaped) as usize;
            }
        }
        assert!(hits > w * h / 10, "the camera missed the model ({hits} hits)");
        println!("{} pixels identical, {hits} on the model, {textured} textured materials", w * h);
    }

    /// The viewport renders with the document's firefly rejection and
    /// indirect clamp, as the final render does. It always rejected and never
    /// clamped, so either setting changed the saved image and not the
    /// preview of it.
    #[test]
    fn the_viewport_renders_with_the_documents_firefly_settings() {
        let device = Device::shared().expect("an Embree device");
        let built = rebuild(
            None,
            RenderRequest { reject_fireflies: false, clamp_sample: Some(4.0), ..request() },
            &device,
        );
        assert!(!built.context.reject_fireflies, "fireflies rejected though the document says not to");
        assert_eq!(built.context.clamp_sample, Some(4.0), "the document's clamp was ignored");
    }

    fn request() -> RenderRequest {
        // The demo scene, which is still the fastest way to get real geometry
        // in front of the renderer in a test.
        let scene =
            tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        RenderRequest {
            camera: Camera::look_at(Vec3::splat(4.0), Vec3::ZERO, Vec3::Y),
            width: 160,
            height: 120,
            config: IntegratorConfig::product(),
            scene: Arc::new(scene),
            scene_generation: 1,
            denoise: true,
            denoise_strength: 1.0,
            denoise_cadence: 1,
            prefilter_guides: None,
            reject_fireflies: true,
            clamp_sample: None,
            interacting: false,
            gpu_solid: false,
            shading: Shading::Rendered,
            // Tequila's own renderer, which these tests are about: the
            // default engine is Cycles on the graphics card, and every test
            // starting a card session of its own at once left one without.
            engine: tequila_scene::render::RenderEngine::Tequila,
            pixels_per_point: 1.0,
        }
    }

    /// A graphics card Cycles can open but not render on - older than the
    /// kernels shipped, or with a driver too old for them, as a work PC's can
    /// be - leaves the viewport on Tequila's renderer, saying why, rather
    /// than black and silent. Made with kernel files no card can load.
    ///
    /// Ignored, to run on its own (`-- --ignored a_card_that_cannot`):
    /// Cycles reads its kernel folder once per process, so the broken one
    /// would leak into every other Cycles test. Needs `TEQUILA_CYCLES` and
    /// `TEQUILA_TEST_CYCLES_GPU`.
    #[test]
    #[ignore]
    fn a_card_that_cannot_render_falls_back_and_says_why() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        if std::env::var_os("TEQUILA_TEST_CYCLES_GPU").is_none() {
            eprintln!("skipped: TEQUILA_TEST_CYCLES_GPU is not set");
            return;
        }
        let broken = std::env::temp_dir().join(format!("tequila-broken-kernels-app-{}", std::process::id()));
        std::fs::create_dir_all(broken.join("lib")).unwrap();
        for name in ["kernel_sm_75.cubin.zst", "kernel_compute_75.ptx.zst", "kernel_optix.ptx.zst", "kernel_optix_shader_raytrace.ptx.zst"] {
            std::fs::write(broken.join("lib").join(name), b"not a kernel").unwrap();
        }
        std::env::set_var("TEQUILA_CYCLES_KERNELS", &broken);

        let renderer = Orchestrator::new();
        let mut req = request();
        req.width = 64;
        req.height = 48;
        req.engine = tequila_scene::render::RenderEngine::CyclesGpu;
        renderer.submit(req);

        let mut seen = 0;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let note = renderer.engine_note();
            if let Some(image) = renderer.take_image(&mut seen) {
                let mean = image.pixels.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>()
                    / (image.pixels.len() / 4 * 3).max(1) as f64;
                if image.samples >= 4 && mean > 20.0 {
                    match note {
                        Some(EngineNote::FellBack { engine, why }) => {
                            assert_eq!(engine, tequila_scene::render::RenderEngine::CyclesGpu);
                            assert!(!why.is_empty(), "it fell back without saying why");
                            println!("fell back: {why}");
                            return;
                        }
                        other => panic!("a picture, but nothing said about the card: {other:?}"),
                    }
                }
            }
            assert!(Instant::now() < deadline, "a minute of a black viewport; the note says {note:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Under Cycles the viewport shows the floor's reflection too: Tequila's,
    /// laid over Cycles' frames as they come. On black, where Cycles' own
    /// catcher could show none, the sphere shows in the floor below it, and
    /// the floor away from it stays black. Skipped where Cycles is not
    /// installed (`TEQUILA_CYCLES`).
    #[test]
    fn the_cycles_viewport_shows_the_floor_reflection() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let shot = |reflections: f32| {
            let renderer = Orchestrator::new();
            let mut req = request();
            req.width = 64;
            req.height = 48;
            req.denoise = false;
            req.engine = tequila_scene::render::RenderEngine::CyclesCpu;
            let mut scene = (*req.scene).clone();
            scene.instances[1].shadow_catcher =
                Some(tequila_render_core::scene::ShadowCatcher { reflections, ..Default::default() });
            scene.background = tequila_render_core::environment::Background::Color(tequila_core::spectrum::Spectrum::ZERO);
            req.scene = Arc::new(scene);
            renderer.submit(req);
            let mut seen = 0;
            let deadline = Instant::now() + Duration::from_secs(90);
            loop {
                if let Some(image) = renderer.take_image(&mut seen) {
                    if image.samples >= 16 && image.width == 64 {
                        return image;
                    }
                }
                assert!(Instant::now() < deadline, "Cycles never drew the viewport");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        // The floor under the sphere: the bottom rows of the view.
        let floor = |image: &DisplayImage| {
            let mut total = 0.0f64;
            for y in 36..48 {
                for x in 0..64 {
                    let p = &image.pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 3];
                    total += f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]);
                }
            }
            total / (12.0 * 64.0 * 3.0)
        };
        let (plain, mirrored) = (floor(&shot(0.0)), floor(&shot(1.0)));
        assert!(mirrored > plain + 5.0, "the floor's reflection did not show under Cycles: {mirrored} against {plain} without");
    }

    /// With the Cycles engine, a Rendered viewport shows Cycles' frames, with
    /// Cycles' sample count, and a camera move brings a different picture.
    /// Skipped where Cycles is not installed (`TEQUILA_CYCLES`).
    #[test]
    fn the_cycles_engine_draws_the_viewport() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let renderer = Orchestrator::new();
        let mut req = request();
        req.width = 64;
        req.height = 48;
        // The processor by default, which every machine with Cycles has;
        // TEQUILA_TEST_CYCLES_GPU tries the graphics card as well.
        req.engine = if std::env::var_os("TEQUILA_TEST_CYCLES_GPU").is_some() {
            tequila_scene::render::RenderEngine::CyclesGpu
        } else {
            tequila_scene::render::RenderEngine::CyclesCpu
        };
        renderer.submit(req.clone());

        let mut seen = 0;
        let mut wait = |what: &str, enough: &dyn Fn(&DisplayImage) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                if let Some(image) = renderer.take_image(&mut seen) {
                    if enough(&image) {
                        return image;
                    }
                }
                assert!(Instant::now() < deadline, "Cycles never drew the viewport {what}");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let first = wait("at first", &|image| image.samples >= 4 && image.width == 64);
        // The colour channels: alpha is 255 whatever the picture.
        let mean = |image: &DisplayImage| {
            image.pixels.chunks_exact(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).sum::<f64>()
                / (image.pixels.len() / 4 * 3) as f64
        };
        assert!(mean(&first) > 20.0, "the viewport came out black: {}", mean(&first));

        req.camera = Camera::look_at(Vec3 { x: -3.0, y: 1.0, z: 2.0 }, Vec3::ZERO, Vec3::Y);
        renderer.submit(req.clone());
        let moved = wait("after the camera moved", &|image| image.samples >= 4 && image.width == 64);
        let difference = first.pixels.iter().zip(&moved.pixels).map(|(a, b)| a.abs_diff(*b) as u64).sum::<u64>();
        assert!(
            difference > 64 * 48,
            "the camera moved and the picture did not: difference {difference}, samples {} then {}, means {} then {}",
            first.samples,
            moved.samples,
            mean(&first),
            mean(&moved)
        );

        // A part dragged with the gizmo: the same scene with the sphere
        // somewhere else, which moves Cycles' object rather than translating.
        let mut dragged = (*req.scene).clone();
        dragged.instances[0].transform = tequila_core::math::Transform::translation(Vec3 { x: 0.6, y: 1.0, z: 0.3 });
        req.scene = Arc::new(dragged);
        req.scene_generation += 1;
        renderer.submit(req);
        let shifted = wait("after the part moved", &|image| image.samples >= 4 && image.width == 64);
        let difference = moved.pixels.iter().zip(&shifted.pixels).map(|(a, b)| a.abs_diff(*b) as u64).sum::<u64>();
        assert!(difference > 64 * 48, "the part moved and the picture did not: difference {difference}");

        let _ = shifted;

        // Selecting the sphere rings it, from Cycles' object pass. Once the
        // render has settled, so the only difference between two pictures
        // is the selection, not more samples.
        let mut generation = renderer.shared.image_generation.load(Ordering::Acquire);
        let mut quiet = Instant::now();
        let deadline = Instant::now() + Duration::from_secs(60);
        while quiet.elapsed() < Duration::from_millis(1500) {
            let now = renderer.shared.image_generation.load(Ordering::Acquire);
            if now != generation {
                generation = now;
                quiet = Instant::now();
            }
            assert!(Instant::now() < deadline, "the render never settled");
            std::thread::sleep(Duration::from_millis(20));
        }
        // Both pictures through the same redraw - a selection change, which
        // develops the frame already there - first selecting an instance
        // that does not exist, then the sphere: only the ring differs.
        let redraw = |selection: Vec<u32>| {
            let before = renderer.shared.image_generation.load(Ordering::Acquire);
            renderer.set_selection(selection);
            let redraw_by = Instant::now() + Duration::from_secs(10);
            while renderer.shared.image_generation.load(Ordering::Acquire) == before {
                assert!(Instant::now() < redraw_by, "selecting did not redraw the view");
                std::thread::sleep(Duration::from_millis(10));
            }
            renderer.shared.image.lock().unwrap().clone()
        };
        let plain = redraw(vec![999]);
        let ringed = redraw(vec![0]);
        let changed = ringed.pixels.chunks_exact(4).zip(plain.pixels.chunks_exact(4)).filter(|(a, b)| a != b).count();
        assert!(changed > 20, "selecting the sphere drew no outline ({changed} pixels changed)");
    }

    /// A material changed in a Cycles viewport - a colour picked, a label
    /// resized - builds that material's shader again and nothing else, and a
    /// part dragged moves it: neither translates the scene again. Both did,
    /// at every step of the mouse, because each snapshot's lights and
    /// textures were new allocations and were compared by pointer; on a
    /// 200 MB model on a Mac, resizing a label closed Tequila. Skipped where
    /// Cycles is not installed (`TEQUILA_CYCLES`).
    #[test]
    fn a_material_edit_rebuilds_only_its_shader_in_the_cycles_viewport() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let mut view = crate::cycles_view::CyclesView::new(tequila_scene::render::RenderEngine::CyclesCpu).expect("a Cycles session");
        let mut req = request();
        req.width = 64;
        req.height = 48;
        req.engine = tequila_scene::render::RenderEngine::CyclesCpu;
        let mut framebuffer = tequila_render_core::framebuffer::Framebuffer::new(64, 48);
        // Red over green in the middle of the frame, where the sphere is, once
        // Cycles has some samples of the scene it was last given.
        let mut redness = |view: &mut crate::cycles_view::CyclesView, req: &RenderRequest| {
            let deadline = Instant::now() + Duration::from_secs(60);
            while !view.sync(req).expect("synced") {
                assert!(Instant::now() < deadline, "Cycles never took the change");
                view.take_frame(&mut framebuffer);
                std::thread::sleep(Duration::from_millis(10));
            }
            loop {
                if let Some(frame) = view.take_frame(&mut framebuffer) {
                    if frame.samples >= 8 && frame.width == 64 {
                        let pixels = framebuffer.pixels();
                        let (r, g) = (18..30)
                            .flat_map(|y| (24..40).map(move |x| y * 64 + x))
                            .fold((0.0f64, 0.0f64), |(r, g), i| (r + f64::from(pixels[i].sum.r), g + f64::from(pixels[i].sum.g)));
                        return r / g.max(1e-9);
                    }
                }
                assert!(Instant::now() < deadline, "Cycles never drew the change");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let before = redness(&mut view, &req);
        assert_eq!(view.translations(), 1);

        // The sphere painted red: one shader again, the meshes left alone.
        let mut painted = (*req.scene).clone();
        let material = painted.instances[0].material.index();
        painted.materials[material].base_color = tequila_core::spectrum::Spectrum { r: 0.8, g: 0.02, b: 0.02 };
        // Rebuilt around the same things, as the application's snapshots are.
        painted.lights = Arc::new((*painted.lights).clone());
        painted.textures = Arc::new((*painted.textures).clone());
        req.scene = Arc::new(painted);
        req.scene_generation += 1;
        let after = redness(&mut view, &req);
        assert!(after > before * 1.5, "the sphere did not turn red: red over green {before} then {after}");
        assert_eq!(view.translations(), 1, "a colour change translated the scene again");

        // And dragged: Cycles' object moves.
        let mut dragged = (*req.scene).clone();
        dragged.instances[0].transform = tequila_core::math::Transform::translation(Vec3 { x: 0.6, y: 1.0, z: 0.3 });
        dragged.lights = Arc::new((*dragged.lights).clone());
        req.scene = Arc::new(dragged);
        req.scene_generation += 1;
        redness(&mut view, &req);
        assert_eq!(view.translations(), 1, "a dragged part translated the scene again");
    }

    /// Orbiting a Cycles viewport shows Cycles' pictures as it goes, each a
    /// picture of the view. It showed none until the mouse stopped: every
    /// move restarted Cycles before its picture of the last was made, and the
    /// first picture after a restart was thrown away as having no samples,
    /// which it had. Skipped where Cycles is not installed (`TEQUILA_CYCLES`).
    #[test]
    fn the_cycles_viewport_shows_pictures_while_orbiting() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let renderer = Orchestrator::new();
        let mut req = request();
        // Big enough that Cycles, on the processor, navigates with pictures
        // smaller than the view.
        req.width = 640;
        req.height = 480;
        // The processor by default, as in the test above.
        req.engine = if std::env::var_os("TEQUILA_TEST_CYCLES_GPU").is_some() {
            tequila_scene::render::RenderEngine::CyclesGpu
        } else {
            tequila_scene::render::RenderEngine::CyclesCpu
        };
        let camera_at = |angle: f32| Camera::look_at(Vec3 { x: 4.0 * angle.sin(), y: 2.0, z: 4.0 * angle.cos() }, Vec3::ZERO, Vec3::Y);
        req.camera = camera_at(0.0);
        renderer.submit(req.clone());
        let mut seen = 0;
        // Cycles starting and the scene translated are not what is tested.
        let deadline = Instant::now() + Duration::from_secs(120);
        while renderer.take_image(&mut seen).is_none() {
            assert!(Instant::now() < deadline, "Cycles never drew the viewport");
            std::thread::sleep(Duration::from_millis(10));
        }

        // Three seconds of a mouse turning the view sixty times a second.
        req.interacting = true;
        let (started, mut moves, mut shown) = (Instant::now(), 0u32, Vec::new());
        while started.elapsed() < Duration::from_secs(3) {
            moves += 1;
            req.camera = camera_at(moves as f32 * 0.01);
            renderer.submit(req.clone());
            let tick = Instant::now();
            while tick.elapsed() < Duration::from_micros(16_667) {
                if let Some(image) = renderer.take_image(&mut seen) {
                    shown.push(image);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        assert!(shown.len() >= 5, "{} pictures in three seconds of orbiting", shown.len());

        // Each a picture of the view at its own size, not a smaller picture
        // read as a larger one: block by block, near a settled one of where
        // the orbit ended.
        req.interacting = false;
        renderer.submit(req);
        let deadline = Instant::now() + Duration::from_secs(60);
        let settled = loop {
            if let Some(image) = renderer.take_image(&mut seen) {
                if image.width == 640 && image.samples >= 16 {
                    break image;
                }
            }
            assert!(Instant::now() < deadline, "the view never settled after the orbit");
            std::thread::sleep(Duration::from_millis(10));
        };
        let blocks = |image: &DisplayImage| -> Vec<f64> {
            let (w, h) = (image.width, image.height);
            let mut sums = vec![0.0; 8 * 6];
            for y in 0..h {
                for x in 0..w {
                    let p = &image.pixels[(y * w + x) * 4..(y * w + x) * 4 + 3];
                    sums[(y * 6 / h) * 8 + x * 8 / w] += (f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])) / 3.0;
                }
            }
            sums.iter().map(|s| s / (w * h / 48) as f64).collect()
        };
        let reference = blocks(&settled);
        for image in shown.iter().rev().take(3) {
            let off = blocks(image).iter().zip(&reference).map(|(a, b)| (a - b).abs()).sum::<f64>() / 48.0;
            assert!(off < 30.0, "a {}x{} picture from the orbit is {off:.0} away from the view", image.width, image.height);
        }
    }

    /// The core contract of §9.1: the UI must get an image without ever
    /// blocking on the renderer.
    #[test]
    fn the_orchestrator_produces_images_without_blocking() {
        let orchestrator = Orchestrator::new();
        orchestrator.submit(request());

        let mut seen = 0u64;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut image = None;
        while Instant::now() < deadline {
            if let Some(i) = orchestrator.take_image(&mut seen) {
                if i.samples > 0 {
                    image = Some(i);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let image = image.expect("no image arrived within 30 s");
        assert!(image.width > 0 && image.height > 0);
        assert_eq!(image.pixels.len(), image.width * image.height * 4);
        assert!(image.triangles > 1000, "scene did not build");
        // The image must not be uniformly black: something has to have been lit.
        let lit = image
            .pixels
            .chunks(4)
            .filter(|p| p[0] > 8 || p[1] > 8)
            .count();
        assert!(
            lit > image.width * image.height / 20,
            "only {lit} pixels received any light"
        );
        // Alpha must be opaque for display.
        assert!(image.pixels.chunks(4).all(|p| p[3] == 255));
    }

    /// Accumulation must actually progress: a second look must be further
    /// converged than the first.
    #[test]
    fn accumulation_progresses_over_time() {
        let orchestrator = Orchestrator::new();
        orchestrator.submit(request());

        let mut seen = 0u64;
        let mut first = 0u32;
        let mut last = 0u32;
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Some(i) = orchestrator.take_image(&mut seen) {
                if first == 0 && i.samples > 0 {
                    first = i.samples;
                }
                last = last.max(i.samples);
                if last > first && first > 0 {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(first > 0, "never rendered anything");
        assert!(last > first, "accumulation stalled at {first} samples");
    }

    /// The outline goes *outside* the selection.
    ///
    /// Which side of the boundary it lands on is the whole design of the
    /// thing: painted inside, it covers the silhouette of the part being
    /// selected, and the one moment a person most needs to see an edge clearly
    /// is the moment they have just clicked it.
    #[test]
    fn the_outline_rings_the_selection_without_covering_it() {
        // A 9x9 image with a 3x3 block of instance 1 in the middle.
        let (w, h) = (9usize, 9usize);
        let mut fb = Framebuffer::new(w, h);
        for y in 3..6 {
            for x in 3..6 {
                fb.pixel_mut(x, y).first_instance = 1;
            }
        }
        let mut pixels = vec![0u8; w * h * 4];
        outline_selection(&mut pixels, &fb, w, h, 1.0, &[1]);

        let ringed = |x: usize, y: usize| pixels[(y * w + x) * 4] > 0;
        // Every pixel of the selection itself is untouched.
        for y in 3..6 {
            for x in 3..6 {
                assert!(!ringed(x, y), "the ring covered the part at ({x}, {y})");
            }
        }
        // The pixels immediately around it are ringed.
        for x in 2..7 {
            assert!(ringed(x, 2), "no ring above the part at x={x}");
            assert!(ringed(x, 6), "no ring below the part at x={x}");
        }
        // And it does not spread across the whole image.
        assert!(
            !ringed(0, 0),
            "the ring reached the far corner of the image"
        );
    }

    /// The ring is as wide in points on a Retina frame as on a standard one.
    /// Ringed in its own pixels, a Cycles frame at the screen's pixels had
    /// half the ring Solid draws around the same part.
    #[test]
    fn the_outline_is_as_wide_in_points_on_a_retina_frame() {
        let ring = |density: f32| {
            let (w, h) = (400usize, 300usize);
            let mut fb = Framebuffer::new(w, h);
            for y in 140..160 {
                for x in 190..210 {
                    fb.pixel_mut(x, y).first_instance = 1;
                }
            }
            let mut pixels = vec![0u8; w * h * 4];
            outline_selection(&mut pixels, &fb, w, h, density, &[1]);
            // Ringed pixels on the row through the middle, left of the part.
            (0..200).filter(|&x| pixels[(150 * w + x) * 4] > 0).count()
        };
        assert_eq!(ring(1.0), 1);
        assert_eq!(ring(2.0), 2);
        assert_eq!(ring(1.84), 2);
    }

    /// The ring is the square neighbourhood of the selection, exactly — the
    /// same pixels the brute-force search over every neighbour found, on a
    /// selection with holes, edges and two separate pieces, at both radii.
    #[test]
    fn the_outline_matches_a_search_of_every_neighbour() {
        for (w, h) in [(40usize, 30usize), (720, 710)] {
            let mut fb = Framebuffer::new(w, h);
            let mut state = 12345u32;
            for y in 0..h {
                for x in 0..w {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    // Two blobs, one against the image's edge, and noise.
                    let blob = ((x as i32 - w as i32 / 3).pow(2) + (y as i32 - h as i32 / 2).pow(2)) < (w as i32 / 6).pow(2);
                    let edge = x < w / 10 && y > h / 2;
                    fb.pixel_mut(x, y).first_instance = if blob || edge { 4 } else if state >> 28 == 0 { 9 } else { 2 };
                }
            }
            let selected = [4u32, 9];
            let mut fast = vec![0u8; w * h * 4];
            outline_selection(&mut fast, &fb, w, h, 1.0, &selected);

            let radius = if w.min(h) > 700 { 2i32 } else { 1i32 };
            let is_selected = |x: i32, y: i32| selected.contains(&fb.pixel(x as usize, y as usize).first_instance);
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    let near = !is_selected(x, y)
                        && (-radius..=radius).any(|dy| {
                            (-radius..=radius).any(|dx| {
                                let (nx, ny) = (x + dx, y + dy);
                                nx >= 0 && ny >= 0 && nx < w as i32 && ny < h as i32 && is_selected(nx, ny)
                            })
                        });
                    let ringed = fast[(y as usize * w + x as usize) * 4] > 0;
                    assert_eq!(ringed, near, "({x}, {y}) in a {w}x{h} image");
                }
            }
        }
    }

    /// Nothing selected, nothing drawn. Worth its own test because the
    /// alternative — a boundary check that treats "no ids" as "every id" — is
    /// a one-character mistake that fills the frame with the ring colour.
    #[test]
    fn an_empty_selection_draws_nothing() {
        let (w, h) = (8usize, 8usize);
        let mut fb = Framebuffer::new(w, h);
        for y in 2..6 {
            for x in 2..6 {
                fb.pixel_mut(x, y).first_instance = 3;
            }
        }
        let mut pixels = vec![0u8; w * h * 4];
        outline_selection(&mut pixels, &fb, w, h, 1.0, &[]);
        assert!(pixels.iter().all(|b| *b == 0), "something was drawn");
    }

    /// Selecting a part must not restart the render.
    ///
    /// This is the reason the selection has a channel of its own instead of a
    /// field on the request. Request equality is what decides whether to
    /// restart, so a selection carried there would throw away every
    /// accumulated sample each time someone clicked a part — and clicking
    /// parts is what a person does constantly while assigning materials.
    #[test]
    fn selecting_does_not_restart_the_render() {
        let orchestrator = Orchestrator::new();
        orchestrator.submit(request());

        // Wait for the top of the resolution ladder, not merely for four
        // samples, and that is the whole reason this loop looks the way it
        // does. §9.2's ladder climbs on elapsed time and clears the
        // accumulation buffer at each rung, so a sample count read partway up
        // it falls again a moment later through no fault of the selection —
        // the renderer is simply filling a bigger buffer now. Waiting for
        // `resolution_divisor == 1` waits for the last rung: `update` clamps
        // there, so nothing can clear the buffer again until a *request*
        // does, which is exactly the thing this test is about. Keyed off four
        // samples alone the assertion below instead measured how loaded the
        // machine was, and failed under a parallel workspace run.
        let mut seen = 0u64;
        let mut before = 0u32;
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Some(i) = orchestrator.take_image(&mut seen) {
                if i.resolution_divisor == 1 && i.samples >= 4 {
                    before = i.samples;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(before >= 4, "never got four samples at full resolution");

        let generation = orchestrator
            .shared
            .request_generation
            .load(Ordering::Acquire);
        orchestrator.set_selection(vec![0]);
        std::thread::sleep(Duration::from_millis(120));

        assert_eq!(
            orchestrator
                .shared
                .request_generation
                .load(Ordering::Acquire),
            generation,
            "selecting bumped the request generation, which restarts the render"
        );
        let image = orchestrator.take_image(&mut seen);
        // Asserted separately so that a ladder that somehow stepped again
        // says so, rather than surfacing as a baffling drop in samples.
        let divisor = image.as_ref().map_or(1, |i| i.resolution_divisor);
        assert_eq!(
            divisor, 1,
            "the resolution ladder stepped after it had already topped out"
        );
        let after = image.map_or(before, |i| i.samples);
        assert!(
            after >= before,
            "samples fell from {before} to {after} after a selection change"
        );
    }

    /// Dropping the orchestrator must stop and join the render thread rather
    /// than leaving it running.
    #[test]
    fn dropping_the_orchestrator_stops_the_thread() {
        let orchestrator = Orchestrator::new();
        orchestrator.submit(request());
        std::thread::sleep(Duration::from_millis(50));
        drop(orchestrator); // joins; would hang if the loop ignored `running`
    }

    #[test]
    fn resubmitting_an_identical_request_is_a_no_op() {
        let orchestrator = Orchestrator::new();
        orchestrator.submit(request());
        let generation = orchestrator
            .shared
            .request_generation
            .load(Ordering::Acquire);
        orchestrator.submit(request());
        assert_eq!(
            orchestrator
                .shared
                .request_generation
                .load(Ordering::Acquire),
            generation,
            "an identical request bumped the generation and would restart the render"
        );
    }

    /// Style changes must not go through the request path, or every exposure
    /// slider drag would throw the accumulated image away.
    #[test]
    fn style_changes_do_not_touch_the_render_request() {
        let orchestrator = Orchestrator::new();
        orchestrator.submit(request());
        let generation = orchestrator
            .shared
            .request_generation
            .load(Ordering::Acquire);
        orchestrator.set_style(ImageStyle {
            exposure: 1.5,
            ..Default::default()
        });
        assert_eq!(
            orchestrator
                .shared
                .request_generation
                .load(Ordering::Acquire),
            generation,
            "a style change restarted the render"
        );
    }

    /// A drag issues a transform change per frame. Rebuilding every per-mesh
    /// BLAS each time is what makes dragging a large assembly stutter, so the
    /// rebuild path has to take the in-place route — and the test asserts it
    /// by identity, since a rebuild would produce a different allocation.
    #[test]
    fn moving_an_instance_updates_the_bvh_rather_than_rebuilding_it() {
        let device = Device::shared().expect("Embree");
        let first = request();
        let built = rebuild(None, first.clone(), &device);
        let original = Arc::as_ptr(&built.intersector);

        let mut moved_scene = (*first.scene).clone();
        for instance in &mut moved_scene.instances {
            instance.transform = tequila_core::math::Transform::translation(Vec3::splat(1.0));
        }
        let moved = RenderRequest {
            scene: Arc::new(moved_scene),
            scene_generation: first.scene_generation + 1,
            ..first.clone()
        };

        let after = rebuild(Some(built), moved, &device);
        assert_eq!(
            Arc::as_ptr(&after.intersector),
            original,
            "a pure transform change rebuilt the whole BVH"
        );
    }

    /// And a change that is not merely a re-placement still rebuilds, or the
    /// BVH would describe geometry that is no longer there.
    #[test]
    fn changing_the_geometry_still_rebuilds() {
        let device = Device::shared().expect("Embree");
        let first = request();
        let built = rebuild(None, first.clone(), &device);
        let original = Arc::as_ptr(&built.intersector);

        // One fewer instance is structural.
        let mut smaller = (*first.scene).clone();
        smaller.instances.truncate(1);
        let changed = RenderRequest {
            scene: Arc::new(smaller),
            scene_generation: first.scene_generation + 1,
            ..first.clone()
        };

        let after = rebuild(Some(built), changed, &device);
        assert_ne!(
            Arc::as_ptr(&after.intersector),
            original,
            "a structural change was applied in place"
        );
    }
}
