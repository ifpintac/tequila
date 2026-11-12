//! The viewport through Cycles.
//!
//! When the engine is Cycles and the viewport is Rendered, a live Cycles
//! session renders the view on its own thread, refining one image, and the
//! render thread shows its newest frame through the same develop, grade and
//! HUD as Tequila's own frames. Tequila's renderer still builds the scene's
//! BVH, because picking a part with the mouse goes through it.
//!
//! What restarts what: a new scene is translated again; an edit to materials
//! alone - a label resized, a colour picked - builds only their shaders again;
//! a part dragged only moves Cycles' objects; and a camera move or a resize
//! only moves Cycles' camera, which is the cheap path and the one orbiting
//! takes sixty times a second.

use crate::orchestrator::RenderRequest;
use tequila_core::spectrum::Spectrum;
use tequila_render_core::camera::Camera;
use tequila_render_core::framebuffer::{Framebuffer, Pixel};
use tequila_render_cycles::{translate, Session};
use tequila_scene::render::RenderEngine;

/// Samples a still view refines to. Adaptive sampling stops most views well
/// before this; the limit is for the few that never converge.
const VIEW_SAMPLES: u32 = 1024;

/// Samples a moving view's picture is asked for: one, which is all it shows.
/// Asked for more, Cycles went on from its one-sample picture to four more
/// samples of a view the next move threw away - 18 ms at 480x270 on the
/// Headphones, which a restart does not cut short - and only then started
/// the next picture.
const MOVING_SAMPLES: u32 = 1;

/// The longest a change waits for Cycles' picture of the one before
/// (`CyclesView::sync`): long enough for a heavy scene's first picture, short
/// enough that a view whose picture never came still follows the mouse.
const HELD_AT_MOST: std::time::Duration = std::time::Duration::from_secs(3);

/// How long the camera has to be still before the view goes back to its
/// full size: long enough that a pause between two strokes of an orbit does
/// not cost a full-size restart, short enough to feel immediate.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

/// How long the view has to have been dragged for its letting go to end the
/// move there and then (`CyclesView::sync`): longer than a wheel's notch,
/// which the window counts as a moment's drag.
const DRAGGED_AT_LEAST: std::time::Duration = std::time::Duration::from_millis(100);

/// How long a picture should take while the camera moves: about thirty a
/// second, which is what an orbit needs to follow the hand.
const NAVIGATION_TARGET: std::time::Duration = std::time::Duration::from_millis(33);

/// The person's own tuning of the two above (Preferences › Viewport): the
/// milliseconds a moving picture is given, and the samples a still view
/// refines to. Read by the render thread, set by the window's.
static TARGET_MS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(NAVIGATION_TARGET.as_millis() as u32);
static SAMPLES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(VIEW_SAMPLES);

/// Set how long a moving picture may take and how far a still view
/// refines. A still view at another limit starts again toward the new one.
pub fn set_tuning(target_ms: u32, samples: u32) {
    use std::sync::atomic::Ordering;
    TARGET_MS.store(target_ms.clamp(8, 200), Ordering::Relaxed);
    SAMPLES.store(samples.clamp(1, 1 << 20), Ordering::Relaxed);
}

fn navigation_target() -> std::time::Duration {
    std::time::Duration::from_millis(u64::from(TARGET_MS.load(std::sync::atomic::Ordering::Relaxed)))
}

fn view_samples() -> u32 {
    SAMPLES.load(std::sync::atomic::Ordering::Relaxed)
}

/// The coarsest picture while moving: its long side no shorter than this.
/// Coarser buys nothing: below it, a picture's time is Cycles' fixed cost
/// of a restart and of launching a sample's kernels - 100x56 pixels took
/// the same 42 ms as 362x203 - and Cycles divides further on its own while
/// it settles on a size.
const NAVIGATION_SMALLEST: usize = 480;

/// A smaller picture while the camera moves, as Blender's and every quick
/// viewport do: Cycles took about 90 ms for each picture after a camera move
/// at the view's size, whatever the model - ten a second, an orbit that
/// stutters - and a quarter of the pixels took 42 ms. The divider is chosen
/// from how long pictures actually take, so a heavy model on a small Mac
/// drops further and a light one stays sharper; when the camera stops, the
/// view goes back to its full size and refines.
#[derive(Clone, Copy, Debug)]
struct Navigation {
    /// The view's size divided by this, on each side, while moving.
    divider: f32,
    /// When the camera last moved.
    moved_at: Option<std::time::Instant>,
    /// Since when the person has been dragging the view, while they are.
    dragging_since: Option<std::time::Instant>,
}

impl Default for Navigation {
    fn default() -> Self {
        Navigation { divider: 2.0, moved_at: None, dragging_since: None }
    }
}

/// The divider after a moving picture at `divider` took `took`: half a step
/// toward the one that would take [`NAVIGATION_TARGET`] - a picture's pixels
/// go as the divider squared - and never so coarse that the long side of a
/// view `long_side` pixels wide falls below [`NAVIGATION_SMALLEST`].
pub fn adjusted_divider(divider: f32, took: std::time::Duration, long_side: usize) -> f32 {
    let ratio = (took.as_secs_f32() / navigation_target().as_secs_f32()).clamp(0.1, 10.0);
    let wanted = divider * ratio.sqrt();
    let most = (long_side as f32 / NAVIGATION_SMALLEST as f32).max(1.0);
    (divider * 0.5 + wanted * 0.5).clamp(1.0, most)
}

/// `full` divided by `divider` on each side, the long side never under
/// [`NAVIGATION_SMALLEST`]: a view smaller than that is not divided at all.
/// The divider is held to that only once a moving picture has come back
/// ([`adjusted_divider`]), and a small view's first one was divided anyway.
pub fn divided(full: (usize, usize), divider: f32) -> (usize, usize) {
    let most = (full.0.max(full.1) as f32 / NAVIGATION_SMALLEST as f32).max(1.0);
    let divider = divider.clamp(1.0, most);
    let side = |n: usize| ((n as f32 / divider).round() as usize).max(1);
    (side(full.0), side(full.1))
}

pub struct CyclesView {
    session: Session,
    engine: RenderEngine,
    /// The scene last translated. Compared by pointer, as the viewport's own
    /// rebuild does: a camera move bumps the document's generation but keeps
    /// the scene, and must move Cycles' camera rather than translate again.
    scene: Option<std::sync::Arc<tequila_render_core::scene::SceneSnapshot>>,
    camera: Option<Camera>,
    /// When the request Cycles last restarted for was made.
    asked: Option<std::time::Instant>,
    /// The size Cycles renders now: `full`, or smaller while the camera
    /// moves.
    size: (usize, usize),
    /// The size the view asked for.
    full: (usize, usize),
    /// The samples Cycles was asked for: [`VIEW_SAMPLES`], or
    /// [`MOVING_SAMPLES`] while the view moves.
    samples: u32,
    /// Whether the picture Cycles is making was asked for as a moving one.
    moving: bool,
    navigation: Navigation,
    frame: tequila_render_cycles::FrameBuffers,
    seen: u64,
    paused: bool,
    /// Textures already on the device, so an edit that leaves them alone does
    /// not send them again.
    images: translate::Images,
    /// The Cycles object of each instance, to move a dragged part without
    /// translating the scene again.
    objects: Vec<Option<tequila_render_cycles::Node>>,
    /// Each material's Cycles shader, to build a changed one again in place.
    shaders: Vec<tequila_render_cycles::Node>,
    /// The lighting the scene was translated with: Ground illumination, the
    /// bounce limits, and the glass shadows pass (which follows from how
    /// caustics are found).
    lighting: (bool, u32, u32, tequila_render_core::settings::ShadowGlass),
    /// Whether frames come with the background left out, for Tequila's to
    /// go under them (`Translation::background_under`).
    background_under: bool,
    /// That background, at the size of the frame it last went under.
    under: Vec<Spectrum>,
    under_size: (usize, usize),
    /// The scene's floor reflects, and Cycles draws none: the viewport lays
    /// Tequila's reflection over the frames (`tequila_render_cpu::floor`).
    floor_reflection: bool,
    /// Whether Cycles has shown a picture since it last restarted, and when
    /// it did: a change waits for the picture of the one before (`sync`).
    answered: bool,
    restarted: std::time::Instant,
    /// How many times the whole scene has been translated.
    translations: u64,
    /// Whether parts imported or deleted go into a session made afresh
    /// (`renews_for_new_parts`), and how many times one was.
    pub(crate) renew_on_new_parts: bool,
    renewals: u64,
}

/// Whether imported or deleted parts go into a session made afresh: on a
/// graphics card with memory of its own, where Cycles' rebuild in place
/// failed (see `CyclesView::sync`). Not on a Mac, and not on the processor.
fn renews_for_new_parts(engine: RenderEngine) -> bool {
    engine == RenderEngine::CyclesGpu && !cfg!(target_os = "macos")
}

impl CyclesView {
    /// A live session on the engine's device. Slow the first time (the GPU
    /// kernels load), so it is made once and kept.
    pub fn new(engine: RenderEngine) -> Result<CyclesView, String> {
        let session = Session::live(engine.cycles_device(), 0, None)?;
        Ok(CyclesView {
            session,
            engine,
            scene: None,
            camera: None,
            asked: None,
            size: (0, 0),
            full: (0, 0),
            samples: 0,
            moving: false,
            navigation: Navigation::default(),
            frame: Default::default(),
            seen: 0,
            paused: false,
            images: translate::Images::default(),
            objects: Vec::new(),
            shaders: Vec::new(),
            lighting: (false, 0, 0, tequila_render_core::settings::ShadowGlass::Clear),
            background_under: false,
            under: Vec::new(),
            under_size: (0, 0),
            floor_reflection: false,
            answered: true,
            restarted: std::time::Instant::now(),
            translations: 0,
            renew_on_new_parts: renews_for_new_parts(engine),
            renewals: 0,
        })
    }

    /// This view again on a new session, the old session ended first so the
    /// two never hold the graphics card at once (an RTX 2060 has 6 GB, and
    /// a session's path states take a good share of it). Everything sent
    /// to the old one - meshes, shaders, pictures - is sent again.
    fn renew(&mut self) -> Result<(), String> {
        let (engine, translations, renew, renewals) = (self.engine, self.translations, self.renew_on_new_parts, self.renewals);
        self.session.close();
        *self = CyclesView::new(engine)?;
        self.translations = translations;
        self.renew_on_new_parts = renew;
        self.renewals = renewals + 1;
        Ok(())
    }

    pub fn engine(&self) -> RenderEngine {
        self.engine
    }

    /// How many times the whole scene has been translated, rather than
    /// updated in place.
    #[cfg(test)]
    pub fn translations(&self) -> u64 {
        self.translations
    }

    /// How many times a fresh session took parts imported or deleted.
    #[cfg(test)]
    pub fn renewals(&self) -> u64 {
        self.renewals
    }

    /// Whether frames need Tequila's floor reflection laid over them.
    pub fn floor_reflection(&self) -> bool {
        self.floor_reflection
    }

    /// Bring Cycles up to `request`, restarting its image if anything it
    /// shows has changed. `true` when it restarted.
    pub fn sync(&mut self, request: &RenderRequest) -> Result<bool, String> {
        let full = (request.width.max(1), request.height.max(1));
        let same = self.scene.as_ref().is_some_and(|scene| std::sync::Arc::ptr_eq(scene, &request.scene));
        // A part dragged with the gizmo changes where instances are and
        // nothing else: Cycles' objects move, and nothing is translated.
        let moved = !same && self.scene.as_ref().is_some_and(|scene| translate::only_moved(scene, &request.scene));
        // An edit to materials alone builds their shaders again and leaves
        // the meshes on the device. Translating the whole scene for each step
        // of a label being resized sent a big model's every mesh to the
        // device and built its BVH again at the speed of the mouse.
        let repainted = (!same && !moved && self.session.can_reopen())
            .then(|| self.scene.as_ref().and_then(|scene| translate::changed_materials(scene, &request.scene)))
            .flatten();
        // Ground illumination decides what the floor is to Cycles, and the
        // bounce limits how far its paths go; how caustics are found, which
        // glass shadows pass and which parts cast and receive caustics.
        let config = &request.config;
        let lighting = (config.ground_illumination, config.max_bounces, config.max_transmission_bounces, config.shadow_glass());
        let relit = lighting != self.lighting;
        let mut rescene = (!same && !moved && repainted.is_none()) || relit;
        let moved = moved && !relit;
        let repainted = repainted.filter(|_| !relit);
        // The camera moved (an orbit, a pan, a zoom, a 3D mouse) or the scene
        // was edited (a material slider, a part dragged with the gizmo): a
        // smaller picture while it goes on, since Cycles no longer divides
        // it itself. A resized view is neither, and starts at its size.
        let now = std::time::Instant::now();
        let changed = self.camera.is_some_and(|camera| camera != request.camera) || rescene || moved || repainted.is_some();
        if changed && self.camera.is_some() && self.full == full {
            self.navigation.moved_at = Some(now);
        }
        // A drag let go of is over: the view goes back to its size then,
        // rather than `SETTLE` after the last move - which kept the first
        // clean picture 150 ms further from the hand letting go.
        let let_go = !request.interacting && self.navigation.dragging_since.is_some_and(|since| now.duration_since(since) >= DRAGGED_AT_LEAST);
        let dragging_since = if request.interacting { self.navigation.dragging_since.or(Some(now)) } else { None };
        if let_go {
            self.navigation.moved_at = None;
        }
        let navigating = request.interacting || self.navigation.moved_at.is_some_and(|at| now.duration_since(at) < SETTLE);
        let size = if navigating { divided(full, self.navigation.divider) } else { full };
        // Still again at the same size - a small view, never divided - goes
        // on from one sample to a still view's.
        let samples = if navigating { MOVING_SAMPLES } else { view_samples() };
        let recamera = self.camera != Some(request.camera) || self.size != size || self.samples != samples;
        if !rescene && !moved && repainted.is_none() && !recamera {
            self.navigation.dragging_since = dragging_since;
            return Ok(false);
        }
        // A change that comes before Cycles has shown its picture of the last
        // one waits for it, as Blender's does (`Session::ready_to_reset`).
        // Restarting at every move of the mouse cancelled each picture before
        // it was made, and an orbit showed nothing until the mouse stopped.
        // The caller asks again, so the newest change is the one that goes in.
        // Not behind a still view's first picture, though: the camera moving
        // again just after it stopped waited for a full-size sample nobody
        // would see - the move's first picture a sample at the view's size late.
        // A drag let go of while held is still one when the change goes in:
        // forgotten here, the mouse's last position went to Cycles as another
        // moving picture and the view settled a moment after it - 150 ms more
        // before the first clean picture, and the more often the busier the
        // graphics card, which is the more often a picture is on its way.
        if request.interacting {
            self.navigation.dragging_since = dragging_since;
        }
        if !self.answered && self.samples == MOVING_SAMPLES && self.restarted.elapsed() < HELD_AT_MOST {
            return Ok(false);
        }
        self.navigation.dragging_since = dragging_since;
        // Parts imported or deleted, on a graphics card with memory of its
        // own (NVIDIA's CUDA and OptiX, AMD's HIP): a session made afresh,
        // the old one ended first, rather than the scene cleared and built
        // again inside the running one. On CUDA that rebuild, done while the
        // view rendered, ended in an illegal-address error and the view fell
        // back to the processor; a fresh session is what entering the
        // Rendered view makes, which works. A Mac's Metal keeps the rebuild
        // in place, which works there and is quicker.
        if rescene && self.renew_on_new_parts && self.scene.as_ref().is_some_and(|scene| translate::parts_came_or_went(scene, &request.scene)) {
            self.renew()?;
        }
        let view = translate::View {
            camera: request.camera,
            width: size.0,
            height: size.1,
            transparent: false,
            ground_illumination: request.config.ground_illumination,
            light_materials: request.config.light_source_materials,
            light_groups: false,
            shadow_glass: request.config.shadow_glass(),
            manifold_caustics: request.config.manifold_caustics(),
        };
        self.session.begin_edit();
        if let Some(changed) = &repainted {
            // A library that cannot rebuild a shader in place, or a shader that
            // would not build, translates everything instead.
            match translate::update_materials(&mut self.session, &request.scene, &self.shaders, changed, &mut self.images, request.config.light_source_materials, request.config.shadow_glass()) {
                Ok(true) => {}
                Ok(false) => rescene = true,
                Err(error) => {
                    eprintln!("Cycles: a material update failed ({error}); translating the scene again");
                    rescene = true;
                }
            }
        }
        let result = if rescene {
            self.translations += 1;
            self.session.clear_scene();
            translate::build_with(&mut self.session, &request.scene, &view, &mut self.images).and_then(|translation| {
                for warning in &translation.warnings {
                    eprintln!("Cycles: {warning}");
                }
                self.objects = translation.objects;
                self.shaders = translation.shaders;
                self.background_under = translation.background_under;
                self.floor_reflection = translation.floor_reflection;
                translate::configure(
                    &mut self.session,
                    // Never Cycles' own denoiser here: it makes a new denoising
                    // device for every frame it denoises, which took ten
                    // seconds a time inside the application. The frame comes
                    // with its guides and the viewport's denoiser, made once,
                    // does it (`orchestrator::publish`).
                    &translate::Quality::with_lighting(true, false, None, &request.config),
                )
            })
        } else {
            let placed = if moved {
                translate::move_objects(&mut self.session, &request.scene, &self.objects)
            } else {
                Ok(())
            };
            placed.and_then(|()| translate::set_camera(&mut self.session, &request.scene, &view).map(|_| ()))
        };
        // A moving picture is not denoised, so its guides stay behind.
        self.session.set_moving(size != full);
        // Ended whatever happened, so the scene lock is never left held.
        self.session.end_edit(size.0, size.1, samples);
        self.answered = false;
        self.restarted = std::time::Instant::now();
        result?;
        self.scene = Some(request.scene.clone());
        self.lighting = lighting;
        self.camera = Some(request.camera);
        self.asked = Some(request.submitted);
        self.size = size;
        self.full = full;
        self.samples = samples;
        self.moving = navigating;
        // Whatever moved, the background behind it is measured again, while
        // Cycles renders (`prepare`), rather than once its picture is in and
        // the next camera waits for it. A first picture of another size (a
        // library whose Cycles still divides it) has it measured again then.
        self.under_size = (0, 0);
        if self.paused {
            self.session.set_paused(true);
        }
        Ok(true)
    }

    /// What the picture Cycles is making needs from Tequila, done while
    /// Cycles makes it: the background that goes under it, measured at its
    /// size. Measured as Cycles restarted, it held up the picture before,
    /// which went up only after.
    pub fn prepare(&mut self) {
        if self.background_under && self.under_size != self.size {
            if let (Some(scene), Some(camera)) = (&self.scene, &self.camera) {
                self.under = crate::profile::time("render: background under", || translate::background(scene, camera, self.size.0, self.size.1));
                self.under_size = self.size;
            }
        }
    }

    /// Put the newest frame into `framebuffer`, resized to it, as pixels whose
    /// mean is the frame and whose sample count is Cycles'. `None` when there
    /// is nothing new.
    pub fn take_frame(&mut self, framebuffer: &mut Framebuffer) -> Option<tequila_render_cycles::Frame> {
        let frame = self.session.latest_frame(&mut self.frame, self.size, &mut self.seen)?;
        // The first picture of a smaller restart: how long it took decides
        // the next one's size.
        if !self.answered && self.size != self.full {
            let long_side = self.full.0.max(self.full.1);
            self.navigation.divider = adjusted_divider(self.navigation.divider, self.restarted.elapsed(), long_side);
        }
        self.answered = true;
        if framebuffer.width != frame.width || framebuffer.height != frame.height {
            framebuffer.resize(frame.width, frame.height);
        }
        let n = frame.samples.max(1);
        let scale = n as f32;
        // At the frame's own size: it is smaller than the view while Cycles
        // refines it after a change.
        if self.background_under && self.under_size != (frame.width, frame.height) {
            if let (Some(scene), Some(camera)) = (&self.scene, &self.camera) {
                self.under = crate::profile::time("render: background under", || translate::background(scene, camera, frame.width, frame.height));
                self.under_size = (frame.width, frame.height);
            }
        }
        let under = (self.background_under && self.under.len() == frame.width * frame.height).then_some(&self.under);
        let buffers = &self.frame;
        // A row per task for a full-size frame, a million pixels and more;
        // a moving picture's on this thread, as a loop shared out waits for
        // its slowest share, and with the computer busy that was tens of
        // milliseconds for a picture one core does in two.
        use rayon::prelude::*;
        let width = frame.width.max(1);
        let shared_out = frame.width * frame.height >= PARALLEL_PIXELS;
        let fill = |(i, pixel): (usize, &mut Pixel)| {
            let rgba = &buffers.rgba[i * 4..i * 4 + 4];
            let albedo = &buffers.albedo[i * 3..i * 3 + 3];
            let normal = &buffers.normal[i * 3..i * 3 + 3];
            // The instance the pixel shows, for the selection outline: the
            // object pass holds its id plus one, and 0 for nothing.
            let object = buffers.object[i];
            let first_instance = if object >= 0.5 { (object.round() as u32).saturating_sub(1) } else { u32::MAX };
            let (colour, coverage) = match under {
                Some(background) => (translate::lay_under(rgba, background[i]), 1.0),
                None => (translate::to_rendering(rgba[0], rgba[1], rgba[2]).sanitized(), rgba[3].clamp(0.0, 1.0)),
            };
            let albedo: Spectrum = translate::to_rendering(albedo[0], albedo[1], albedo[2]).sanitized();
            // The guide's own frame (Cycles' world, Z up): a denoiser asks only
            // that neighbouring normals be comparable, not which way is up.
            let normal = tequila_core::math::Vec3 { x: normal[0], y: normal[1], z: normal[2] };
            *pixel = Pixel {
                sum: colour * scale,
                albedo_sum: albedo * scale,
                normal_sum: normal * scale,
                coverage_sum: coverage * scale,
                samples: n,
                first_instance,
                ..Pixel::default()
            };
        };
        if shared_out {
            framebuffer.pixels_mut().par_chunks_mut(width).enumerate().for_each(|(y, row)| {
                for (x, pixel) in row.iter_mut().enumerate() {
                    fill((y * width + x, pixel));
                }
            });
        } else {
            framebuffer.pixels_mut().iter_mut().enumerate().for_each(fill);
        }
        Some(frame)
    }

    /// The camera Cycles renders from now.
    pub fn camera(&self) -> Option<Camera> {
        self.camera
    }

    /// When the request Cycles renders now was made.
    pub fn asked(&self) -> Option<std::time::Instant> {
        self.asked
    }

    /// Whether the view is following a moving camera: a smaller picture, or
    /// the camera moved a moment ago. Its pictures go up as they are.
    pub fn navigating(&self) -> bool {
        self.size != self.full || self.navigation.moved_at.is_some_and(|at| at.elapsed() < SETTLE)
    }

    /// Whether the picture Cycles is making now was asked for as a moving
    /// one: of one sample, gone as soon as the next camera is in. Unlike
    /// [`navigating`](Self::navigating) it holds for the picture, not for
    /// the moment, so the floor reflection traced while Cycles makes it and
    /// the one laid over it once it is in are of the same kind.
    pub fn moving_picture(&self) -> bool {
        self.moving
    }

    /// Whether a picture of the last restart is still to come: the render
    /// loop then looks for it often.
    pub fn awaiting_frame(&self) -> bool {
        !self.answered
    }

    /// The size the view asked for.
    pub fn full_size(&self) -> (usize, usize) {
        self.full
    }

    /// The size Cycles is asked to render now.
    pub fn size(&self) -> (usize, usize) {
        self.size
    }


    /// The samples Cycles is asked for now.
    #[cfg(test)]
    pub fn traced_samples(&self) -> u32 {
        self.samples
    }

    /// What Cycles says it is doing, or why it stopped.
    pub fn status(&self) -> Result<String, String> {
        self.session.status()
    }

    /// Stop or resume using the device, keeping the image.
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.paused = paused;
            self.session.set_paused(paused);
        }
    }
}

/// A picture this many pixels or more is worked on by every core; a
/// smaller one, a moving picture, by the render thread alone
/// (`CyclesView::take_frame`).
pub const PARALLEL_PIXELS: usize = 512 * 1024;

/// What a floor reflection layer is of: the scene (by pointer), the camera
/// and the size it is traced at ([`floor_size`]).
pub type FloorKey = (usize, Camera, usize, usize);

/// The size the floor reflection under a picture of `size` is traced at:
/// the picture's own for a still view's, and half of it each way for a
/// moving picture's, which is gone as soon as the next camera is in.
///
/// Measured on the Headphones sample, a moving picture's reflection over
/// its whole frame - 700x400 on an M3 Max, 480x270 on an RTX 2060's PC -
/// took 22 ms of every 36 ms picture on the Mac and 37 ms on the PC, where
/// it alone held orbiting to fifteen pictures a second with the graphics
/// card two-thirds idle. Half of each side is a quarter of the rays: the
/// reflection in a moving picture, up for a thirtieth of a second, is a
/// little softer but where it should be, and the still view's, which
/// refines, is traced at its full size as it always was.
pub fn floor_size(size: (usize, usize), moving: bool) -> (usize, usize) {
    if moving {
        (size.0.div_ceil(2).max(1), size.1.div_ceil(2).max(1))
    } else {
        size
    }
}

/// A floor reflection layer as it falls on the pixels of a picture: one of
/// the picture's own size pixel for pixel, as a still view's is, and a
/// moving picture's half-size one ([`floor_size`]) read between its four
/// nearest pixels - premultiplied, as the layer is, so the reflection's
/// edge fades out rather than darkening.
pub enum Reflection<'a> {
    Same(&'a Framebuffer),
    Scaled {
        /// The picture's width and height.
        size: (usize, usize),
        /// The layer's width and height.
        layer: (usize, usize),
        /// Each of the layer's pixels: its premultiplied colour and share.
        texels: Vec<(Spectrum, f32)>,
    },
}

impl<'a> Reflection<'a> {
    pub fn over(layer: &'a Framebuffer, size: (usize, usize)) -> Reflection<'a> {
        if (layer.width, layer.height) == size {
            return Reflection::Same(layer);
        }
        // A pixel with no share lays nothing, as a still view's does not, so
        // it brings no colour to its neighbours either.
        let texels = layer
            .pixels()
            .iter()
            .map(|p| match p.alpha() {
                share if share > 0.0 => (p.mean(), share),
                _ => (Spectrum::ZERO, 0.0),
            })
            .collect();
        Reflection::Scaled { size, layer: (layer.width, layer.height), texels }
    }

    /// The reflection's colour, premultiplied, and its share of pixel `i`
    /// of the picture.
    #[inline]
    pub fn at(&self, i: usize) -> (Spectrum, f32) {
        match self {
            Reflection::Same(layer) => {
                let pixel = &layer.pixels()[i];
                (pixel.mean(), pixel.alpha())
            }
            Reflection::Scaled { size, layer, texels } => {
                if texels.is_empty() {
                    return (Spectrum::ZERO, 0.0);
                }
                let (x, y) = (i % size.0, i / size.0);
                // Where the pixel's centre falls among the layer's.
                let between = |at: usize, to: usize, from: usize| {
                    let p = ((at as f32 + 0.5) * from as f32 / to as f32 - 0.5).clamp(0.0, (from - 1) as f32);
                    let low = p.floor() as usize;
                    (low, (low + 1).min(from - 1), p - low as f32)
                };
                let (x0, x1, tx) = between(x, size.0, layer.0);
                let (y0, y1, ty) = between(y, size.1, layer.1);
                let at = |x: usize, y: usize| texels[y * layer.0 + x];
                let corners = [(at(x0, y0), (1.0 - tx) * (1.0 - ty)), (at(x1, y0), tx * (1.0 - ty)), (at(x0, y1), (1.0 - tx) * ty), (at(x1, y1), tx * ty)];
                if corners.iter().all(|((_, share), _)| *share <= 0.0) {
                    return (Spectrum::ZERO, 0.0);
                }
                corners.iter().fold((Spectrum::ZERO, 0.0), |(colour, share), ((c, s), w)| (colour + *c * *w, share + s * w))
            }
        }
    }
}

/// The most samples a still view's floor reflection takes: past them it is
/// as clean as the picture it goes over.
const FLOOR_SAMPLES: u32 = 256;

/// Tequila's floor reflection under Cycles' pictures
/// (`tequila_render_cpu::floor`), a layer for each view it goes over: the
/// one Cycles is making now, traced while Cycles renders it, and the one
/// before.
///
/// Measured on the Headphones sample at a 1448x814 view, a sample of the
/// reflection over the whole frame took 90 to 120 ms (200 to 300 in the
/// application, beside Cycles), for the 3% of the pixels that show any
/// of it: the rest are a camera ray and a mirror ray each, a million of them.
/// So once a pass over the whole frame has found where the reflection is,
/// the samples after it are traced only there; and the first picture at a
/// view's full size, as the camera stops, takes its first sample where the
/// moving pictures of the same view found it, so the clean picture is not
/// kept waiting a whole frame's pass. Its second sample is over the whole
/// frame again, and finds the reflection's place at that size for the rest.
#[derive(Default)]
pub struct FloorLayers {
    layers: Vec<FloorLayer>,
}

struct FloorLayer {
    key: FloorKey,
    layer: Framebuffer,
    /// Samples it has.
    passes: u32,
    /// Where it shows a reflection, found by a pass over the whole frame at
    /// its size: the only place it is traced after.
    band: Option<tequila_render_cpu::floor::Band>,
    /// Where another size of the same view showed one, for its first pass.
    guess: Option<tequila_render_cpu::floor::Band>,
    /// Traced for the picture Cycles was making, and not yet laid over it.
    fresh: bool,
}

impl FloorLayers {
    pub fn clear(&mut self) {
        self.layers.clear();
    }

    /// The first sample of the layer for `key`, if it has none, while Cycles
    /// makes the picture it goes over. A layer given up for `stop` is
    /// forgotten, to be begun again.
    pub fn prepare(&mut self, intersector: &dyn tequila_render_core::intersect::Intersector, key: FloorKey, config: &tequila_render_core::settings::IntegratorConfig, stop: &(dyn Fn() -> bool + Sync)) {
        let at = self.find_or_make(key);
        if self.layers[at].passes > 0 {
            return;
        }
        if crate::profile::time("render: floor reflection", || trace(&mut self.layers[at], intersector, config, stop)) {
            self.layers[at].fresh = true;
        } else {
            self.layers.remove(at);
        }
    }

    /// The layer to lay over the picture of `key`, with a sample more unless
    /// it was traced for this very picture, the view is moving (its
    /// pictures are of one sample), or it has all it takes. A sample given
    /// up for `stop` leaves what it had.
    pub fn for_frame(&mut self, intersector: &dyn tequila_render_core::intersect::Intersector, key: FloorKey, config: &tequila_render_core::settings::IntegratorConfig, moving: bool, stop: &(dyn Fn() -> bool + Sync)) -> &Framebuffer {
        let at = self.find_or_make(key);
        let entry = &mut self.layers[at];
        let wanted = entry.passes == 0 || (!entry.fresh && !moving && entry.passes < FLOOR_SAMPLES);
        entry.fresh = false;
        if wanted {
            crate::profile::time("render: floor reflection", || trace(entry, intersector, config, stop));
        }
        &self.layers[at].layer
    }

    /// The layer for `key`, made if there is none: begun where a layer of
    /// the same view at another size found the reflection. Two are kept,
    /// the newest last: a full-size layer is 100 MB.
    fn find_or_make(&mut self, key: FloorKey) -> usize {
        if let Some(at) = self.layers.iter().position(|l| l.key == key) {
            return at;
        }
        let guess = self.layers.iter().rev().find(|l| l.key.0 == key.0 && l.key.1 == key.1 && l.passes > 0).and_then(|l| l.band.map(|band| scaled(band, (l.key.2, l.key.3), (key.2, key.3))));
        self.layers.retain(|l| l.key.0 == key.0);
        if self.layers.len() >= 2 {
            self.layers.remove(0);
        }
        self.layers.push(FloorLayer { key, layer: Framebuffer::new(key.2, key.3), passes: 0, band: None, guess, fresh: false });
        self.layers.len() - 1
    }
}

/// One sample of a layer: where its band says, or where the guess says on
/// its first, or over the whole frame, after which the band is known.
fn trace(entry: &mut FloorLayer, intersector: &dyn tequila_render_core::intersect::Intersector, config: &tequila_render_core::settings::IntegratorConfig, stop: &(dyn Fn() -> bool + Sync)) -> bool {
    use tequila_render_cpu::floor::Band;
    let known = entry.band.or(if entry.passes == 0 { entry.guess } else { None });
    let region = match known {
        Some(Band::Nowhere) => {
            entry.passes += 1;
            return true;
        }
        Some(Band::Within(tile)) => Some(tile),
        Some(Band::Everywhere) | None => None,
    };
    let camera = entry.key.1;
    let started = std::time::Instant::now();
    if !tequila_render_cpu::floor::render_floor_reflection_in(intersector, &mut entry.layer, camera, config, region, stop) {
        crate::view_timing::trace(format_args!("floor reflection {}x{} given up after {:.1} ms", entry.key.2, entry.key.3, started.elapsed().as_secs_f64() * 1000.0));
        return false;
    }
    crate::view_timing::trace(format_args!(
        "floor reflection {}x{} sample {} {} in {:.1} ms",
        entry.key.2,
        entry.key.3,
        entry.passes + 1,
        match region {
            Some(t) => format!("within {}x{}", t.width(), t.height()),
            None => "over the whole frame".to_string(),
        },
        started.elapsed().as_secs_f64() * 1000.0
    ));
    entry.passes += 1;
    if entry.band.is_none() && region.is_none() {
        entry.band = Some(tequila_render_cpu::floor::reflected_band(&entry.layer));
    }
    true
}

/// A band found at size `from` as it falls on size `to`, with a margin of
/// a fiftieth of the long side, as the band's own.
fn scaled(band: tequila_render_cpu::floor::Band, from: (usize, usize), to: (usize, usize)) -> tequila_render_cpu::floor::Band {
    use tequila_render_cpu::floor::Band;
    let Band::Within(tile) = band else { return band };
    let (sx, sy) = (to.0 as f32 / from.0.max(1) as f32, to.1 as f32 / from.1.max(1) as f32);
    let margin = (to.0.max(to.1) / 50).max(8);
    Band::Within(tequila_render_core::scheduler::Tile {
        x0: ((tile.x0 as f32 * sx).floor() as usize).saturating_sub(margin),
        y0: ((tile.y0 as f32 * sy).floor() as usize).saturating_sub(margin),
        x1: ((tile.x1 as f32 * sx).ceil() as usize + margin).min(to.0),
        y1: ((tile.y1 as f32 * sy).ceil() as usize + margin).min(to.1),
    })
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    use std::time::Duration;

    /// Too slow a picture makes the next one smaller, a quick one larger,
    /// half a step at a time; never finer than the view, never so coarse
    /// that the long side falls under 480 pixels.
    #[test]
    fn the_moving_picture_is_sized_to_its_time() {
        let slow = adjusted_divider(2.0, Duration::from_millis(90), 1920);
        assert!(slow > 2.0, "90 ms at 1/2 asks for a smaller picture: {slow}");
        let quick = adjusted_divider(4.0, Duration::from_millis(10), 1920);
        assert!(quick < 4.0, "10 ms at 1/4 can afford a larger one: {quick}");
        assert_eq!(adjusted_divider(1.0, Duration::from_millis(1), 1920), 1.0, "never finer than the view");
        let mut d = 2.0;
        for _ in 0..40 {
            d = adjusted_divider(d, Duration::from_millis(500), 1920);
        }
        assert!((d - 4.0).abs() < 1e-3, "the coarsest is 480 pixels across 1920: {d}");
        // A picture whose time goes as its pixels settles where it takes
        // the target: 132 ms at full size, so 33 ms at half.
        let mut d = 1.0f32;
        for _ in 0..30 {
            let took = Duration::from_secs_f32(0.132 / (d * d));
            d = adjusted_divider(d, took, 1920);
        }
        assert!((d - 2.0).abs() < 0.05, "settled at {d}");
        assert_eq!(divided((1920, 1080), 3.0), (640, 360));
        assert_eq!(divided((1920, 1080), 8.0), (480, 270), "never under 480 pixels across");
        assert_eq!(divided((64, 48), 2.0), (64, 48), "a view smaller than that is not divided");
    }
}

#[cfg(test)]
mod floor_tests {
    use super::*;
    use tequila_core::math::Vec3;

    fn mirrored_demo() -> tequila_render_core::scene::SceneSnapshot {
        let mut scene = tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        scene.instances[1].shadow_catcher = Some(tequila_render_core::scene::ShadowCatcher { reflections: 1.0, ..Default::default() });
        scene
    }

    /// A still view's floor reflection, traced only where its first pass
    /// found it, comes to the same picture as one traced over the whole
    /// frame at every sample; and the first sample of another size of the
    /// same view goes where the first size found it.
    #[test]
    fn the_floor_reflection_is_traced_where_it_shows() {
        let device = tequila_render_cpu::Device::shared().unwrap();
        let intersector = tequila_render_cpu::EmbreeIntersector::build(&device, mirrored_demo()).unwrap();
        let config = tequila_render_core::settings::IntegratorConfig::product();
        let camera = Camera::look_at(Vec3::splat(4.0), Vec3::ZERO, Vec3::Y);
        let mut layers = FloorLayers::default();
        let key = (1usize, camera, 96, 72);
        for _ in 0..6 {
            layers.for_frame(&intersector, key, &config, false, &|| false);
        }
        let banded = layers.for_frame(&intersector, key, &config, true, &|| false).clone();
        let band = layers.layers[0].band.expect("found by the first pass");
        let tequila_render_cpu::floor::Band::Within(tile) = band else { panic!("the reflection is a band under the sphere, not {band:?}") };
        assert!(tile.pixel_count() < 96 * 72, "the band is smaller than the frame");
        let mut whole = Framebuffer::new(96, 72);
        tequila_render_cpu::floor::render_floor_reflection(&intersector, &mut whole, camera, &config, 6);
        assert!(whole.pixels().iter().any(|p| p.alpha() > 0.0), "the floor reflects the sphere");
        for (a, b) in banded.pixels().iter().zip(whole.pixels()) {
            assert!((a.alpha() - b.alpha()).abs() < 1e-5 && a.mean().to_array().iter().zip(b.mean().to_array()).all(|(x, y)| (x - y).abs() < 1e-4), "the banded reflection differs from the whole one");
        }

        // The same view twice the size: begun within where the first found it.
        layers.prepare(&intersector, (1, camera, 192, 144), &config, &|| false);
        let larger = layers.layers.last().unwrap();
        assert_eq!(larger.passes, 1);
        assert!(matches!(larger.guess, Some(tequila_render_cpu::floor::Band::Within(_))), "guessed from the first size");
        assert!(larger.band.is_none(), "not found here until a pass over the whole frame");
        assert!(larger.layer.pixels().iter().any(|p| p.alpha() > 0.0), "and the reflection is there");
        assert!(larger.layer.pixels().iter().any(|p| p.samples == 0), "only within the guess");
    }

    /// A moving picture's reflection is traced at half its size each way,
    /// and laid over the whole picture: about where, and about as strong
    /// as, the reflection traced at the picture's own size, with no pixel
    /// stronger than any of its four. A still picture's layer is laid
    /// pixel for pixel.
    #[test]
    fn a_moving_pictures_reflection_is_traced_at_half_its_size() {
        assert_eq!(floor_size((96, 72), true), (48, 36));
        assert_eq!(floor_size((97, 73), true), (49, 37), "an odd side rounds up");
        assert_eq!(floor_size((1, 1), true), (1, 1));
        assert_eq!(floor_size((96, 72), false), (96, 72), "a still view's at its own size");

        let device = tequila_render_cpu::Device::shared().unwrap();
        let intersector = tequila_render_cpu::EmbreeIntersector::build(&device, mirrored_demo()).unwrap();
        let config = tequila_render_core::settings::IntegratorConfig::product();
        let camera = Camera::look_at(Vec3::splat(4.0), Vec3::ZERO, Vec3::Y);
        let traced = |width, height| {
            let mut layer = Framebuffer::new(width, height);
            for _ in 0..16 {
                tequila_render_cpu::floor::render_floor_reflection_in(&intersector, &mut layer, camera, &config, None, &|| false);
            }
            layer
        };
        let (full, half) = (traced(96, 72), traced(48, 36));
        let own = Reflection::over(&full, (96, 72));
        assert!(matches!(own, Reflection::Same(_)));
        for (i, pixel) in full.pixels().iter().enumerate() {
            assert_eq!(own.at(i), (pixel.mean(), pixel.alpha()), "a layer of the picture's size is laid as it is");
        }
        let laid = Reflection::over(&half, (96, 72));
        let (mut wanted, mut got, mut wanted_area, mut got_area) = (0.0, 0.0, 0usize, 0usize);
        for (i, pixel) in full.pixels().iter().enumerate() {
            let (colour, share) = laid.at(i);
            wanted += pixel.alpha();
            got += share;
            wanted_area += usize::from(pixel.alpha() > 0.0);
            got_area += usize::from(share > 0.0);
            let strongest = half.pixels().iter().map(|p| p.alpha()).fold(0.0, f32::max);
            assert!(share <= strongest + 1e-6 && colour.to_array().iter().all(|c| c.is_finite() && *c >= 0.0));
        }
        assert!(wanted > 0.0, "the floor reflects the sphere");
        assert!((got - wanted).abs() < wanted * 0.15, "as strong overall: {got} laid from half size, {wanted} traced at full");
        assert!(got_area.abs_diff(wanted_area) < wanted_area / 4 + 8, "over about the same pixels: {got_area} and {wanted_area}");
    }

    /// The camera stops after a move: the still view's reflection, begun
    /// where the moving pictures' half-size layers found it, comes to the
    /// very one traced over the whole frame at every sample, bit for bit,
    /// wherever it shows.
    #[test]
    fn the_still_view_after_a_move_has_the_reflection_it_always_had() {
        let device = tequila_render_cpu::Device::shared().unwrap();
        let intersector = tequila_render_cpu::EmbreeIntersector::build(&device, mirrored_demo()).unwrap();
        let config = tequila_render_core::settings::IntegratorConfig::product();
        let orbit = |turn: f32| Camera::look_at(Vec3 { x: 4.0 * turn.cos() + 1.0, y: 4.0, z: 4.0 * turn.sin() + 1.0 }, Vec3::ZERO, Vec3::Y);
        let mut layers = FloorLayers::default();
        let (full, moving) = ((192, 144), (96, 72));
        for step in 0..6 {
            let camera = orbit(step as f32 * 0.05);
            let (width, height) = floor_size(moving, true);
            layers.prepare(&intersector, (1, camera, width, height), &config, &|| false);
            let layer = layers.for_frame(&intersector, (1, camera, width, height), &config, true, &|| false);
            assert_eq!((layer.width, layer.height, layer.pixels().iter().map(|p| p.samples).max()), (48, 36, Some(1)), "one sample at half size");
        }
        let camera = orbit(0.25);
        let key = (1, camera, full.0, full.1);
        assert_eq!(floor_size(full, false), full);
        for _ in 0..8 {
            layers.for_frame(&intersector, key, &config, false, &|| false);
        }
        let still = layers.for_frame(&intersector, key, &config, true, &|| false).clone();
        let begun = layers.layers.last().unwrap();
        assert!(matches!(begun.guess, Some(tequila_render_cpu::floor::Band::Within(_))), "begun where the moving pictures found it");
        assert_eq!(begun.passes, 8);
        let mut whole = Framebuffer::new(full.0, full.1);
        for _ in 0..8 {
            tequila_render_cpu::floor::render_floor_reflection_in(&intersector, &mut whole, camera, &config, None, &|| false);
        }
        let mut shown = 0;
        for (a, b) in still.pixels().iter().zip(whole.pixels()) {
            if a.alpha() > 0.0 || b.alpha() > 0.0 {
                shown += 1;
                assert_eq!((a.sum.to_array(), a.coverage_sum, a.samples), (b.sum.to_array(), b.coverage_sum, b.samples), "the still view's reflection is the whole frame's");
            }
        }
        assert!(shown > 100, "the floor reflects the sphere: {shown} pixels");
    }

    /// A camera moving on before Cycles has shown a still view's first
    /// picture goes in at once; one moving on before Cycles has shown a
    /// moving picture waits for it, so a moving view shows anything at all.
    /// Skipped where Cycles is not installed (`TEQUILA_CYCLES`).
    #[test]
    fn a_move_is_not_held_behind_a_still_pictures_first_sample() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let mut view = CyclesView::new(RenderEngine::CyclesCpu).expect("a Cycles session");
        let scene = std::sync::Arc::new(tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2));
        let at = |a: f32| Camera::look_at(Vec3 { x: 4.0 * a.cos(), y: 4.0, z: 4.0 * a.sin() }, Vec3::ZERO, Vec3::Y);
        let mut req = RenderRequest {
            camera: at(0.0),
            width: 1000,
            height: 750,
            config: tequila_render_core::settings::IntegratorConfig::product(),
            scene,
            scene_generation: 1,
            denoise: false,
            denoise_strength: 1.0,
            denoise_cadence: 1,
            prefilter_guides: None,
            reject_fireflies: true,
            clamp_sample: None,
            interacting: false,
            gpu_solid: false,
            shading: crate::orchestrator::Shading::Rendered,
            engine: RenderEngine::CyclesCpu,
            pixels_per_point: 1.0,
            checker: None,
            submitted: std::time::Instant::now(),
        };
        assert!(view.sync(&req).expect("synced"), "the first view goes in");
        req.camera = at(0.3);
        assert!(view.sync(&req).expect("synced"), "a move goes in before the still view's first picture");
        assert_eq!(view.traced_samples(), MOVING_SAMPLES);
        req.camera = at(0.6);
        assert!(!view.sync(&req).expect("synced"), "the next waits for the moving picture");
    }

    /// A drag let go of puts the view back to its full size at once; a
    /// wheel's notch, a moment's drag, waits for the camera to settle.
    /// Skipped where Cycles is not installed (`TEQUILA_CYCLES`).
    #[test]
    fn letting_go_of_a_drag_goes_back_to_full_size_at_once() {
        if let Err(why) = tequila_render_cycles::available() {
            eprintln!("skipped: {why}");
            return;
        }
        let mut view = CyclesView::new(RenderEngine::CyclesCpu).expect("a Cycles session");
        let at = |a: f32| Camera::look_at(Vec3 { x: 4.0 * a.cos(), y: 4.0, z: 4.0 * a.sin() }, Vec3::ZERO, Vec3::Y);
        let mut req = RenderRequest {
            camera: at(0.0),
            width: 1000,
            height: 750,
            config: tequila_render_core::settings::IntegratorConfig::product(),
            scene: std::sync::Arc::new(tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2)),
            scene_generation: 1,
            denoise: false,
            denoise_strength: 1.0,
            denoise_cadence: 1,
            prefilter_guides: None,
            reject_fireflies: true,
            clamp_sample: None,
            interacting: false,
            gpu_solid: false,
            shading: crate::orchestrator::Shading::Rendered,
            engine: RenderEngine::CyclesCpu,
            pixels_per_point: 1.0,
            checker: None,
            submitted: std::time::Instant::now(),
        };
        let mut framebuffer = Framebuffer::new(1, 1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut settle = |view: &mut CyclesView, req: &RenderRequest| {
            view.sync(req).expect("synced");
            while view.awaiting_frame() {
                assert!(std::time::Instant::now() < deadline, "Cycles never drew the change");
                view.take_frame(&mut framebuffer);
                std::thread::sleep(std::time::Duration::from_millis(2));
                view.sync(req).expect("synced");
            }
        };
        settle(&mut view, &req);
        req.interacting = true;
        req.camera = at(0.3);
        settle(&mut view, &req);
        assert!(view.size().0 < 1000, "moving, a smaller picture");
        std::thread::sleep(std::time::Duration::from_millis(120));
        req.interacting = false;
        view.sync(&req).expect("synced");
        assert!(!view.navigating(), "let go, the move is over");
        assert_eq!(view.size(), (1000, 750), "and the view is at its size at once");

        // Let go while the move's last picture is still being made: the
        // let-go waits behind it, as any change does, and then the view is
        // at its size at once - not after another moving picture of where
        // the mouse stopped and a moment of settling.
        req.interacting = true;
        req.camera = at(0.35);
        view.sync(&req).expect("synced");
        assert!(view.moving_picture() && view.awaiting_frame(), "a moving picture on its way");
        std::thread::sleep(std::time::Duration::from_millis(120));
        req.interacting = false;
        req.camera = at(0.4);
        assert!(!view.sync(&req).expect("synced"), "held behind the moving picture");
        let mut shown = Framebuffer::new(1, 1);
        while view.awaiting_frame() {
            assert!(std::time::Instant::now() < deadline, "Cycles never drew the move");
            view.take_frame(&mut shown);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(view.sync(&req).expect("synced"), "the let-go goes in");
        assert!(!view.navigating() && !view.moving_picture(), "let go behind a picture, the move is over");
        assert_eq!((view.size(), view.camera()), ((1000, 750), Some(at(0.4))), "at its size, where the mouse stopped");
        settle(&mut view, &req);

        // A notch: moving, and a frame later not, still settling. (Not
        // waiting for its picture, which on a busy processor takes longer
        // than a notch.)
        req.interacting = true;
        req.camera = at(0.6);
        view.sync(&req).expect("synced");
        req.interacting = false;
        view.sync(&req).expect("synced");
        assert!(view.navigating(), "a moment's drag still settles");
    }
}

