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

/// The longest a change waits for Cycles' picture of the one before
/// (`CyclesView::sync`): long enough for a heavy scene's first picture, short
/// enough that a view whose picture never came still follows the mouse.
const HELD_AT_MOST: std::time::Duration = std::time::Duration::from_secs(3);

/// How long the camera has to be still before the view goes back to its
/// full size: long enough that a pause between two strokes of an orbit does
/// not cost a full-size restart, short enough to feel immediate.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

/// How long a picture should take while the camera moves: about thirty a
/// second, which is what an orbit needs to follow the hand.
const NAVIGATION_TARGET: std::time::Duration = std::time::Duration::from_millis(33);

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
}

impl Default for Navigation {
    fn default() -> Self {
        Navigation { divider: 2.0, moved_at: None }
    }
}

/// The divider after a moving picture at `divider` took `took`: half a step
/// toward the one that would take [`NAVIGATION_TARGET`] - a picture's pixels
/// go as the divider squared - and never so coarse that the long side of a
/// view `long_side` pixels wide falls below [`NAVIGATION_SMALLEST`].
pub fn adjusted_divider(divider: f32, took: std::time::Duration, long_side: usize) -> f32 {
    let ratio = (took.as_secs_f32() / NAVIGATION_TARGET.as_secs_f32()).clamp(0.1, 10.0);
    let wanted = divider * ratio.sqrt();
    let most = (long_side as f32 / NAVIGATION_SMALLEST as f32).max(1.0);
    (divider * 0.5 + wanted * 0.5).clamp(1.0, most)
}

/// `full` divided by `divider` on each side.
pub fn divided(full: (usize, usize), divider: f32) -> (usize, usize) {
    let side = |n: usize| ((n as f32 / divider.max(1.0)).round() as usize).max(1);
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
    /// The size Cycles renders now: `full`, or smaller while the camera
    /// moves.
    size: (usize, usize),
    /// The size the view asked for.
    full: (usize, usize),
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
            size: (0, 0),
            full: (0, 0),
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
        // The camera moved, and nothing else about the view: an orbit, a
        // pan, a zoom, a 3D mouse. A part dragged with the gizmo is the
        // mouse held down, `interacting`.
        let now = std::time::Instant::now();
        if self.camera.is_some_and(|camera| camera != request.camera) && self.full == full {
            self.navigation.moved_at = Some(now);
        }
        let navigating = request.interacting || self.navigation.moved_at.is_some_and(|at| now.duration_since(at) < SETTLE);
        let size = if navigating { divided(full, self.navigation.divider) } else { full };
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
        let recamera = self.camera != Some(request.camera) || self.size != size;
        if !rescene && !moved && repainted.is_none() && !recamera {
            return Ok(false);
        }
        // A change that comes before Cycles has shown its picture of the last
        // one waits for it, as Blender's does (`Session::ready_to_reset`).
        // Restarting at every move of the mouse cancelled each picture before
        // it was made, and an orbit showed nothing until the mouse stopped.
        // The caller asks again, so the newest change is the one that goes in.
        if !self.answered && self.restarted.elapsed() < HELD_AT_MOST {
            return Ok(false);
        }
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
        // Ended whatever happened, so the scene lock is never left held.
        self.session.end_edit(size.0, size.1, VIEW_SAMPLES);
        self.answered = false;
        self.restarted = std::time::Instant::now();
        result?;
        self.scene = Some(request.scene.clone());
        self.lighting = lighting;
        // Whatever moved, the background behind it is measured again.
        self.under_size = (0, 0);
        self.camera = Some(request.camera);
        self.size = size;
        self.full = full;
        if self.paused {
            self.session.set_paused(true);
        }
        Ok(true)
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
        for (i, pixel) in framebuffer.pixels_mut().iter_mut().enumerate() {
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
        }
        Some(frame)
    }

    /// Whether the view is following a moving camera: a smaller picture, or
    /// the camera moved a moment ago. Its pictures go up as they are.
    pub fn navigating(&self) -> bool {
        self.size != self.full || self.navigation.moved_at.is_some_and(|at| at.elapsed() < SETTLE)
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
    #[cfg(test)]
    pub fn traced_size(&self) -> (usize, usize) {
        self.size
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

#[cfg(test)]
mod navigation_tests {
    use super::*;
    use std::time::Duration;

    /// Too slow a picture makes the next one smaller, a quick one larger,
    /// half a step at a time; never finer than the view, never so coarse
    /// that the long side falls under 200 pixels.
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
        assert_eq!(divided((7, 5), 10.0), (1, 1));
    }
}
