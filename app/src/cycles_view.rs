//! The viewport through Cycles.
//!
//! When the engine is Cycles and the viewport is Rendered, a live Cycles
//! session renders the view on its own thread, refining one image, and the
//! render thread shows its newest frame through the same develop, grade and
//! HUD as Tequila's own frames. Tequila's renderer still builds the scene's
//! BVH, because picking a part with the mouse goes through it.
//!
//! What restarts what: a new scene (any document edit) is translated again; a
//! camera move or a resize only moves Cycles' camera, which is the cheap path
//! and the one orbiting takes sixty times a second.

use crate::orchestrator::RenderRequest;
use tequila_core::spectrum::Spectrum;
use tequila_render_core::camera::Camera;
use tequila_render_core::framebuffer::{Framebuffer, Pixel};
use tequila_render_cycles::{translate, Session};
use tequila_scene::render::RenderEngine;

/// Samples a still view refines to. Adaptive sampling stops most views well
/// before this; the limit is for the few that never converge.
const VIEW_SAMPLES: u32 = 1024;

pub struct CyclesView {
    session: Session,
    engine: RenderEngine,
    /// The scene last translated. Compared by pointer, as the viewport's own
    /// rebuild does: a camera move bumps the document's generation but keeps
    /// the scene, and must move Cycles' camera rather than translate again.
    scene: Option<std::sync::Arc<tequila_render_core::scene::SceneSnapshot>>,
    camera: Option<Camera>,
    size: (usize, usize),
    frame: tequila_render_cycles::FrameBuffers,
    seen: u64,
    paused: bool,
    /// Textures already on the device, so an edit that leaves them alone does
    /// not send them again.
    images: translate::Images,
    /// The Cycles object of each instance, to move a dragged part without
    /// translating the scene again.
    objects: Vec<Option<tequila_render_cycles::Node>>,
    /// The lighting the scene was translated with: Ground illumination, and
    /// the bounce limits.
    lighting: (bool, u32, u32),
    /// Whether frames come with the background left out, for Tequila's to
    /// go under them (`Translation::background_under`).
    background_under: bool,
    /// That background, at the size of the frame it last went under.
    under: Vec<Spectrum>,
    under_size: (usize, usize),
    /// The scene's floor reflects, and Cycles draws none: the viewport lays
    /// Tequila's reflection over the frames (`tequila_render_cpu::floor`).
    floor_reflection: bool,
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
            frame: Default::default(),
            seen: 0,
            paused: false,
            images: translate::Images::default(),
            objects: Vec::new(),
            lighting: (false, 0, 0),
            background_under: false,
            under: Vec::new(),
            under_size: (0, 0),
            floor_reflection: false,
        })
    }

    pub fn engine(&self) -> RenderEngine {
        self.engine
    }

    /// Whether frames need Tequila's floor reflection laid over them.
    pub fn floor_reflection(&self) -> bool {
        self.floor_reflection
    }

    /// Bring Cycles up to `request`, restarting its image if anything it
    /// shows has changed. `true` when it restarted.
    pub fn sync(&mut self, request: &RenderRequest) -> Result<bool, String> {
        let size = (request.width.max(1), request.height.max(1));
        let same = self.scene.as_ref().is_some_and(|scene| std::sync::Arc::ptr_eq(scene, &request.scene));
        // A part dragged with the gizmo changes where instances are and
        // nothing else: Cycles' objects move, and nothing is translated.
        let moved = !same && self.scene.as_ref().is_some_and(|scene| translate::only_moved(scene, &request.scene));
        // Ground illumination decides what the floor is to Cycles, and the
        // bounce limits how far its paths go.
        let config = &request.config;
        let lighting = (config.ground_illumination, config.max_bounces, config.max_transmission_bounces);
        let relit = lighting != self.lighting;
        let rescene = (!same && !moved) || relit;
        let moved = moved && !relit;
        let recamera = self.camera != Some(request.camera) || self.size != size;
        if !rescene && !moved && !recamera {
            return Ok(false);
        }
        let view = translate::View {
            camera: request.camera,
            width: size.0,
            height: size.1,
            transparent: false,
            ground_illumination: request.config.ground_illumination,
        };
        self.session.begin_edit();
        let result = if rescene {
            self.session.clear_scene();
            translate::build_with(&mut self.session, &request.scene, &view, &mut self.images).and_then(|translation| {
                for warning in &translation.warnings {
                    eprintln!("Cycles: {warning}");
                }
                self.objects = translation.objects;
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
        result?;
        self.scene = Some(request.scene.clone());
        self.lighting = lighting;
        // Whatever moved, the background behind it is measured again.
        self.under_size = (0, 0);
        self.camera = Some(request.camera);
        self.size = size;
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
        if framebuffer.width != frame.width || framebuffer.height != frame.height {
            framebuffer.resize(frame.width, frame.height);
        }
        let n = frame.samples.max(1);
        let scale = n as f32;
        // At the frame's own size: it is smaller than the view while Cycles
        // refines it after a change.
        if self.background_under && self.under_size != (frame.width, frame.height) {
            if let (Some(scene), Some(camera)) = (&self.scene, &self.camera) {
                self.under = translate::background(scene, camera, frame.width, frame.height);
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
