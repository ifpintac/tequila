//! The Snapshots tab (called Studios in the code and the files): saved shots, model sets and material variants
//! (`TEQUILA_SPEC.md` §1.12, `docs/02` §8.1).
//!
//! The three are one tab because they are one job — keeping several
//! versions of a product shot in one project and moving between them — and
//! a Studio is made of the other two. Everything here edits the document
//! through [`tequila_scene::studio`]'s commands, so every change is one
//! undo step and survives a save.

use crate::project::Project;
use eframe::egui;
use tequila_scene::studio::{ModelSet, ModelSets, MultiMaterial, Studio};
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::Command;

#[derive(Default)]
pub struct StudioTab {
    wizard: crate::configurator_panel::Wizard,
    thumbnails: Thumbnails,
    reorder: Reorder,
    /// What the configurator asked for this frame - to present, or to
    /// render every combination - drained by the application.
    pub action: Option<crate::configurator_panel::Action>,
    /// Scroll the configurator into view on the next frame.
    pub reveal: bool,
    /// Render every Studio, one image each, asked for this frame.
    pub render_all: bool,
    /// Render the turntable's frames, first to last, asked for this frame.
    pub render_frames: Option<(u32, u32)>,
    turntable: Turntable,
}

/// A turntable as the Turntable section sets one up, before it is made.
struct Turntable {
    frames: u32,
    degrees: f32,
    fps: f32,
    ease: bool,
    /// The frame to show.
    frame: u32,
}

impl Default for Turntable {
    fn default() -> Self {
        Turntable { frames: 36, degrees: 360.0, fps: 24.0, ease: false, frame: 0 }
    }
}

impl StudioTab {
    /// Returns true when a Studio moved the camera, so the viewport's own
    /// camera has to follow.
    pub fn show(&mut self, ui: &mut egui::Ui, project: &mut Project) -> bool {
        let mut camera_moved = false;
        camera_moved |= studios(ui, project, &mut self.thumbnails, &mut self.reorder);
        let views = project.document().studios.len();
        if views > 1
            && ui
                .button(format!("Render all {views} Snapshots"))
                .on_hover_text("Queue every Snapshot, one image each, named for the view: {view} in the file name.")
                .clicked()
        {
            self.render_all = true;
        }
        // The Snapshots are the page's head; under them four tools that
        // build on them, each folded to a row until it is in use, in the
        // person's own order. Open, each began with a paragraph, and the
        // page was mostly reading.
        ui.add_space(8.0);
        let document = project.document();
        let (turning, sets, variants_in_use) = (document.animation.is_some(), document.model_sets.sets.len(), document.multi_materials.len());
        let reveal = std::mem::take(&mut self.reveal);
        let configured = !project.document().configurator.options.is_empty();
        let before = project.document().configurator.current.clone();
        let mut action = None;
        let mut page = crate::sections::Page::new(ui, "snapshots");
        for key in page.order() {
            match key {
                "turntable" => page.section(ui, key, "Turntable", |ui| {
                    crate::field::Fold::new("Turntable").id_salt("snapshots-turntable").default_open(turning).show(ui, |ui| {
                        camera_moved |= self.turntable(ui, project);
                    });
                }),
                "model_sets" => page.section(ui, key, "Model sets", |ui| {
                    crate::field::Fold::new("Model sets")
                        .id_salt("snapshots-model-sets")
                        .aside(if sets > 0 { format!("{sets}") } else { String::new() })
                        .default_open(sets > 0)
                        .show(ui, |ui| model_sets(ui, project));
                }),
                "variants" => page.section(ui, key, "Material variants", |ui| {
                    crate::field::Fold::new("Material variants")
                        .id_salt("snapshots-variants")
                        .aside(if variants_in_use > 0 { format!("{variants_in_use}") } else { String::new() })
                        .default_open(variants_in_use > 0)
                        .show(ui, |ui| variants(ui, project));
                }),
                "configurator" => page.section(ui, key, "Configurator", |ui| {
                    let wizard = &mut self.wizard;
                    let fold = crate::field::Fold::new("Configurator").id_salt("snapshots-configurator").default_open(configured).reveal(reveal).show(ui, |ui| {
                        action = wizard.show(ui, project);
                    });
                    if reveal {
                        ui.scroll_to_rect(fold.header_response.rect, Some(egui::Align::TOP));
                    }
                }),
                _ => {}
            }
        }
        page.finish(ui);
        self.action = action.or(self.action.take());
        // A configuration with a Studio in it can move the camera.
        let configurator = &project.document().configurator;
        if configurator.current != before
            && configurator.options.iter().any(|o| o.drives == tequila_scene::configurator::Drives::Studio)
        {
            camera_moved = true;
        }
        camera_moved
    }

    /// Hold Studio `from` as though it were being dragged to `slot` (0 is
    /// above the first, the count below the last), for a picture of a drag
    /// part-way: the drop line and the Studio under the pointer are drawn
    /// and nothing moves. `None` lets it go.
    pub fn stage_drag(&mut self, drag: Option<(usize, usize)>) {
        self.reorder.staged = drag;
    }

    /// How many Studios show a picture of the scene as it is now, and
    /// whether one is being rendered.
    pub fn pictures(&self, project: &Project) -> (usize, bool) {
        (self.thumbnails.current(project), self.thumbnails.job.is_some())
    }
}

impl StudioTab {
    /// The product turning over frames: made from a count, an angle and a
    /// rate, a frame shown, and every frame rendered. Returns true when
    /// showing a frame turned the product.
    fn turntable(&mut self, ui: &mut egui::Ui, project: &mut Project) -> bool {
        use tequila_scene::animation::{Animation, Interpolation};
        let draft = &mut self.turntable;
        crate::theme::note(ui, "The product turning on the spot, rendered as one numbered image a frame.");
        ui.add(crate::field::Slider::new(&mut draft.frames, 1..=360).text("Frames").logarithmic(true).clamping(egui::SliderClamping::Never))
            .on_hover_text("How many images the turn is made of. 36 is a step of ten degrees.");
        draft.frames = draft.frames.clamp(1, 3600);
        ui.add(crate::field::Slider::new(&mut draft.degrees, -360.0..=360.0).text("Turn").suffix("°").fixed_decimals(0).clamping(egui::SliderClamping::Never))
            .on_hover_text("How far it turns over the frames. 360 is once round; a negative angle turns the other way.");
        draft.degrees = draft.degrees.clamp(-3600.0, 3600.0);
        ui.add(crate::field::Slider::new(&mut draft.fps, 1.0..=60.0).text("Frames a second").fixed_decimals(0).clamping(egui::SliderClamping::Never))
            .on_hover_text("How fast it plays as a video.");
        draft.fps = draft.fps.clamp(1.0, 120.0);
        ui.add(crate::field::Checkbox::new(&mut draft.ease, "Ease in and out"))
            .on_hover_text("Start and stop the turn gently, rather than at an even rate all the way.");
        let existing = project.document().animation.clone();
        ui.horizontal(|ui| {
            let label = if existing.is_some() { "Replace turntable" } else { "Make turntable" };
            if ui
                .button(label)
                .on_hover_text("The product turning through the angle over the frames, the frame after the last where the first began. One undo takes it back.")
                .clicked()
            {
                let mut animation = Animation::turntable(draft.frames, draft.fps, draft.degrees);
                if draft.ease {
                    animation.interpolation = Interpolation::Ease;
                }
                let _ = project.apply(Command::SetAnimation { animation: Some(Box::new(animation)) });
            }
            if existing.is_some() && ui.button("Remove").clicked() {
                let _ = project.apply(Command::SetAnimation { animation: None });
            }
        });
        let Some(animation) = existing else {
            return false;
        };
        let last = animation.frames.saturating_sub(1);
        let mut moved = false;
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut draft.frame).range(0..=last).prefix("Frame "));
            if ui.button("Show").on_hover_text("Turn the product to this frame. Undo turns it back.").clicked() {
                if let Some(yaw) = animation.yaw_at(draft.frame.min(last)) {
                    moved = project.apply(Command::TurnProduct { yaw }).is_ok();
                }
            }
        });
        if ui
            .button(format!("Render {} frames", animation.frames))
            .on_hover_text("Queue every frame, one image each, numbered by {frame} in the file name; the Render window renders a range. A frame already rendered is left out.")
            .clicked()
        {
            self.render_frames = Some((0, last));
        }
        moved
    }
}

/// The parts among the selection and everything under it.
fn selected_parts(project: &Project) -> Vec<NodeId> {
    let document = project.document();
    let mut out = Vec::new();
    for node in project.selection.nodes() {
        for id in document.tree.descendants_and_self(node) {
            if document.tree.get(id).is_some_and(|n| matches!(n.kind, NodeKind::Part { .. })) && !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

fn report(project: &mut Project, result: Result<(), tequila_scene::command::CommandError>) {
    if let Err(error) = result {
        project.status = error.to_string();
    }
}

// ---- Studios --------------------------------------------------------------------

/// Each Studio's picture: the shot it recalls, rendered small on another
/// thread, one at a time (`TEQUILA_SPEC.md` §1.12, "auto thumbnails").
///
/// A picture is kept under its Studio's
/// [`picture_key`](Studio::picture_key) - what the Studio holds, not its
/// name - in the project ([`Project::keep_studio_picture`]), which is saved
/// with them, so a project opened again shows them at once. A Studio that
/// changes has another key and is rendered again at once. One whose scene
/// changed under it - a material edited, another Studio applied that set
/// what this one leaves alone - keeps its picture until the scene has been
/// still for a moment, and is rendered again then, so a slider's drag does
/// not start a render at every step.
#[derive(Default)]
struct Thumbnails {
    /// The pictures shown, by key.
    shown: std::collections::HashMap<u64, Shown>,
    /// The picture being rendered.
    job: Option<Job>,
    /// Ones that would not render, so they are not tried every frame.
    failed: std::collections::HashSet<u64>,
    /// Studios' environments, apart from the scene's.
    environment: tequila_scene::environment::EnvironmentCache,
    /// The project the pictures are of ([`Project::document_epoch`]);
    /// `None` before the first look.
    epoch: Option<u64>,
    /// What each Studio's picture would be made from now, and since when
    /// that has been so.
    wanted: Vec<u64>,
    still_since: Option<std::time::Instant>,
}

/// A picture shown: the texture, and what it was made from
/// ([`made_from`]).
struct Shown {
    texture: egui::TextureHandle,
    made_from: u64,
}

/// A picture being rendered on another thread.
struct Job {
    key: u64,
    made_from: u64,
    epoch: u64,
    /// The Studio's place in the list when it started, for the spinner.
    index: usize,
    render: std::thread::JoinHandle<Option<egui::ColorImage>>,
}

/// A thumbnail's width in pixels; its height follows the render's shape.
const THUMBNAIL_WIDTH: usize = 192;

/// How long the scene stays still before pictures it has left out of date
/// are rendered again.
const STILL: std::time::Duration = std::time::Duration::from_millis(800);

/// What a Studio's picture is made from besides the Studio: the scene as
/// it is, for whatever the Studio leaves as it is - camera, environment,
/// look, model set, the product's turn, the variants and lights it does
/// not set - every other edit to the scene, and the render's shape. A
/// picture made from anything else is out of date.
fn made_from(studio: &Studio, now: &Studio, project: &Project) -> u64 {
    use std::fmt::Write;
    use std::hash::{Hash, Hasher};
    let render = &project.document().render;
    let mut text = format!("{} {} {}x{}", project.document_epoch(), project.scene_edits(), render.width, render.height);
    if studio.camera.is_none() {
        let _ = write!(text, " {:?}", now.camera);
    }
    if studio.environment.is_none() {
        let _ = write!(text, " {:?}", now.environment);
    }
    if studio.image_style.is_none() {
        let _ = write!(text, " {:?}", now.image_style);
    }
    if studio.model_set.is_none() {
        let _ = write!(text, " {:?}", now.model_set);
    }
    if studio.product_yaw.is_none() {
        let _ = write!(text, " {:?}", now.product_yaw);
    }
    for (id, active) in &now.variants {
        if !studio.variants.iter().any(|(set, _)| set == id) {
            let _ = write!(text, " {id}:{active}");
        }
    }
    for light in &now.lights {
        if !studio.lights.iter().any(|o| o.target == light.target) {
            let _ = write!(text, " {light:?}");
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    studio.picture_key().hash(&mut hasher);
    text.hash(&mut hasher);
    hasher.finish()
}

impl Thumbnails {
    /// Take a finished render, show the pictures the project was opened
    /// with, and start the next render wanted: a Studio with no picture
    /// first, then, once the scene has been still a moment, one whose
    /// picture is out of date.
    fn update(&mut self, ctx: &egui::Context, project: &mut Project) {
        let opened = self.epoch != Some(project.document_epoch());
        if opened {
            // Another project: these pictures are of nothing in it. A
            // render still running is let finish and thrown away.
            self.epoch = Some(project.document_epoch());
            self.shown.clear();
            self.failed.clear();
            self.wanted.clear();
        }
        if self.job.as_ref().is_some_and(|job| job.render.is_finished()) {
            if let Some(job) = self.job.take() {
                let image = job.render.join().ok().flatten();
                if Some(job.epoch) == self.epoch {
                    match image {
                        Some(image) => {
                            let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                            if let Some(png) = crate::thumbnail::encode_png(image.size[0], image.size[1], &rgba) {
                                project.keep_studio_picture(job.key, std::sync::Arc::new(png));
                            }
                            let texture = ctx.load_texture(format!("studio-{:x}", job.key), image, egui::TextureOptions::LINEAR);
                            self.shown.insert(job.key, Shown { texture, made_from: job.made_from });
                        }
                        None => {
                            self.failed.insert(job.key);
                        }
                    }
                }
            }
        }

        let now = Studio::capture("", project.document());
        let studios = &project.document().studios;
        let keys: Vec<u64> = studios.iter().map(Studio::picture_key).collect();
        let wanted: Vec<u64> = studios.iter().map(|s| made_from(s, &now, project)).collect();
        if wanted != self.wanted || self.still_since.is_none() {
            self.wanted = wanted.clone();
            self.still_since = Some(std::time::Instant::now());
        }
        // Kept with the project and not shown yet: shown at once. Opened
        // with it, it is a picture of the scene as it is; one rendered
        // before an undo brought its Studio back may not be, and is
        // rendered again once the scene is still.
        for (index, &key) in keys.iter().enumerate() {
            if self.shown.contains_key(&key) {
                continue;
            }
            let image = project.document().studio_pictures.get(key).and_then(|png| crate::thumbnail::decode_png(png.as_slice()));
            if let Some(image) = image {
                let texture = ctx.load_texture(format!("studio-{key:x}"), image, egui::TextureOptions::LINEAR);
                self.shown.insert(key, Shown { texture, made_from: if opened { wanted[index] } else { 0 } });
            }
        }
        // Textures of Studios gone or changed are let go; the project keeps
        // their pictures a while, for an undo.
        self.shown.retain(|key, _| keys.contains(key));
        if self.job.is_some() {
            return;
        }

        let usable = |index: &usize| !self.failed.contains(&keys[*index]);
        let missing = (0..keys.len()).filter(usable).find(|&i| !self.shown.contains_key(&keys[i]));
        let stale = (0..keys.len()).filter(usable).find(|&i| self.shown.get(&keys[i]).is_some_and(|s| s.made_from != wanted[i]));
        let still = self.still_since.map_or(STILL, |t| t.elapsed());
        let next = match (missing, stale) {
            (Some(index), _) => index,
            (None, Some(index)) if still >= STILL => index,
            (None, Some(_)) => {
                ctx.request_repaint_after(STILL - still);
                return;
            }
            (None, None) => return,
        };
        // An environment of the Studio's own is made on another thread
        // first; the picture waits for it.
        if !project.studio_environment_ready(next, &mut self.environment) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let key = keys[next];
        let Some((scene, camera, style, config)) = project.studio_scene(next, &mut self.environment) else {
            self.failed.insert(key);
            return;
        };
        let render = &project.document().render;
        let height = ((THUMBNAIL_WIDTH as f32 * render.height.max(1) as f32 / render.width.max(1) as f32).round() as usize).clamp(32, 256);
        let repaint = ctx.clone();
        let job = std::thread::Builder::new().name("tequila-studio-thumbnail".into()).spawn(move || {
            let image = render_thumbnail(scene, camera, style, config, THUMBNAIL_WIDTH, height);
            repaint.request_repaint();
            image
        });
        if let Ok(render) = job {
            self.job = Some(Job { key, made_from: wanted[next], epoch: project.document_epoch(), index: next, render });
        }
    }

    /// How many Studios show a picture made from the scene as it is.
    fn current(&self, project: &Project) -> usize {
        let studios = &project.document().studios;
        studios
            .iter()
            .zip(&self.wanted)
            .filter(|(studio, wanted)| self.shown.get(&studio.picture_key()).is_some_and(|s| s.made_from == **wanted))
            .count()
    }
}

/// The shot, small: a few samples a pixel, denoised and developed in the
/// Studio's own image style. On one worker, so the viewport keeps the rest
/// of the machine.
fn render_thumbnail(
    scene: std::sync::Arc<tequila_render_core::scene::SceneSnapshot>,
    camera: tequila_render_core::camera::Camera,
    style: tequila_color::ImageStyle,
    config: tequila_render_core::settings::IntegratorConfig,
    width: usize,
    height: usize,
) -> Option<egui::ColorImage> {
    use tequila_render_core::framebuffer::Framebuffer;
    use tequila_render_core::scheduler::ProgressiveSchedule;
    use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    let pool = POOL
        .get_or_init(|| rayon::ThreadPoolBuilder::new().num_threads(2).thread_name(|_| "tequila-studio-cpu".into()).build().ok())
        .as_ref()?;
    pool.install(|| {
        let device = Device::shared().ok()?;
        let intersector = EmbreeIntersector::build(&device, (*scene).clone()).ok()?;
        let mut framebuffer = Framebuffer::new(width, height);
        let context = RenderContext { camera, config, ..RenderContext::default() };
        let mut schedule = ProgressiveSchedule::default();
        schedule.update(60_000);
        render_pass(&intersector, &mut framebuffer, &context, &schedule, 24);
        let guides = framebuffer.to_denoiser_buffers();
        let denoised = tequila_denoise::Denoiser::shared().ok().and_then(|denoiser| {
            denoiser
                .denoise(width, height, &guides.color, &guides.albedo, &guides.normal, tequila_denoise::DenoiseSettings::default())
                .ok()
        });
        let colour: Vec<tequila_core::spectrum::Spectrum> = (0..width * height)
            .map(|i| match &denoised {
                Some(image) => tequila_core::spectrum::Spectrum::from_rendering_rgb(tequila_core::math::vec3(
                    image.data[i * 3],
                    image.data[i * 3 + 1],
                    image.data[i * 3 + 2],
                )),
                None => framebuffer.pixel(i % width, i / width).mean(),
            })
            .collect();
        let rgba = style.develop(width, height, &colour);
        Some(egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba))
    })
}

/// A Studio being dragged to another place in the list, by its picture or
/// its grip: the line where it would land, and the Studio under the
/// pointer. Let go there, it moves, as one undo step; Escape puts it back.
/// The Up and Down buttons do the same a place at a time, for a keyboard.
#[derive(Default)]
struct Reorder {
    /// The Studio being dragged.
    dragging: Option<usize>,
    /// Each Studio's place on screen this frame, top to bottom.
    rows: Vec<egui::Rect>,
    /// A drag held for a screenshot: the Studio, and where it would land.
    staged: Option<(usize, usize)>,
}

/// Where a Studio dragged to height `y` lands among `rows`: before the
/// first row whose middle is below it, 0 above them all and the count
/// below them all.
fn drop_slot(rows: &[egui::Rect], y: f32) -> usize {
    rows.iter().take_while(|row| row.center().y < y).count()
}

/// `list` with the item at `from` moved to `slot` (counted before the
/// move), or `None` where that leaves it where it was.
fn moved<T>(mut list: Vec<T>, from: usize, slot: usize) -> Option<Vec<T>> {
    if from >= list.len() || slot > list.len() || slot == from || slot == from + 1 {
        return None;
    }
    let item = list.remove(from);
    list.insert(if slot > from { slot - 1 } else { slot }, item);
    Some(list)
}

impl Reorder {
    /// Start dragging Studio `index` when `response` - its picture or its
    /// grip - begins a drag.
    fn grab(&mut self, response: &egui::Response, index: usize) {
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.dragging = Some(index);
        }
        if response.dragged() || response.hovered() && self.dragging.is_none() {
            response.ctx.set_cursor_icon(if response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
    }

    /// Draw the drag under way over the list just laid out - the Studio
    /// lifted, the line where it would land, the Studio under the pointer -
    /// and say, when it is let go somewhere new, which Studio moves to
    /// which slot.
    fn show(&mut self, ui: &egui::Ui, studios: &[Studio], picture: impl Fn(usize) -> Option<egui::TextureHandle>) -> Option<(usize, usize)> {
        let count = self.rows.len();
        if self.dragging.is_some_and(|from| from >= count) {
            self.dragging = None;
        }
        let pointer = ui.ctx().pointer_interact_pos().or_else(|| ui.ctx().pointer_hover_pos());
        let (from, slot, at) = match (self.dragging, self.staged) {
            (Some(from), _) => {
                let at = pointer?;
                (from, drop_slot(&self.rows, at.y), at)
            }
            (None, Some((from, slot))) if from < count && slot <= count => {
                // Held for a screenshot: the pointer a little inside the
                // slot, as a person's would be.
                let y = match slot {
                    0 => self.rows[0].top() + 12.0,
                    s if s == count => self.rows[count - 1].bottom() - 12.0,
                    s => self.rows[s].top() + 4.0,
                };
                (from, slot, egui::pos2(self.rows[from].left() + 60.0, y))
            }
            _ => return None,
        };
        if self.dragging.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dragging = None;
            return None;
        }
        let p = crate::theme::p();
        let painter = ui.painter();
        // The Studio lifted out of its place.
        painter.rect_filled(self.rows[from].expand2(egui::vec2(3.0, 0.0)), 6.0, p.accent_fill.gamma_multiply(0.6));
        // Where it would land: a line across the gap, with a dot at each
        // end, unless that is where it already is.
        let lands = slot != from && slot != from + 1;
        if lands {
            let y = match slot {
                0 => self.rows[0].top() - 1.0,
                s if s == count => self.rows[count - 1].bottom() + 3.0,
                s => (self.rows[s - 1].bottom() + self.rows[s].top()) * 0.5 + 1.0,
            };
            let (left, right) = (self.rows[0].left(), self.rows.iter().map(|r| r.right()).fold(self.rows[0].right(), f32::max));
            painter.line_segment([egui::pos2(left + 4.0, y), egui::pos2(right - 4.0, y)], egui::Stroke::new(2.5_f32, p.accent));
            painter.circle_filled(egui::pos2(left + 4.0, y), 4.0, p.accent);
            painter.circle_filled(egui::pos2(right - 4.0, y), 4.0, p.accent);
        }
        // The Studio under the pointer: its picture and name, on top of
        // everything.
        let layer = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.id().with("studio-drag")));
        let name = studios.get(from).map_or(String::new(), |s| s.name.clone());
        let picture = picture(from);
        let width = 120.0;
        let image_height = picture.as_ref().map_or(0.0, |t| width * t.size_vec2().y / t.size_vec2().x.max(1.0));
        let card = egui::Rect::from_min_size(at + egui::vec2(14.0, 6.0), egui::vec2(width + 8.0, image_height + 26.0));
        layer.add(egui::epaint::Shadow { offset: egui::vec2(0.0, 4.0), blur: 12.0, spread: 0.0, color: egui::Color32::from_black_alpha(60) }.as_shape(card, 7.0));
        layer.rect(card, 7.0, p.panel, egui::Stroke::new(1.0_f32, p.accent));
        if let Some(texture) = picture {
            let image = egui::Rect::from_min_size(card.min + egui::vec2(4.0, 4.0), egui::vec2(width, image_height));
            layer.image(texture.id(), image, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        layer.text(
            egui::pos2(card.left() + 6.0, card.bottom() - 11.0),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(crate::theme::BODY - 0.5),
            p.text,
        );
        if self.dragging.is_none() {
            return None;
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        // Near the top or bottom of the panel, the list scrolls to show
        // more of itself.
        let clip = ui.clip_rect();
        let edge = 28.0;
        if at.y < clip.top() + edge {
            ui.scroll_with_delta(egui::vec2(0.0, (clip.top() + edge - at.y).min(edge) * 0.4));
            ui.ctx().request_repaint();
        } else if at.y > clip.bottom() - edge {
            ui.scroll_with_delta(egui::vec2(0.0, -(at.y - (clip.bottom() - edge)).min(edge) * 0.4));
            ui.ctx().request_repaint();
        }
        if ui.input(|i| i.pointer.any_released()) {
            self.dragging = None;
            return lands.then_some((from, slot));
        }
        None
    }
}

fn studios(ui: &mut egui::Ui, project: &mut Project, thumbnails: &mut Thumbnails, reorder: &mut Reorder) -> bool {
    thumbnails.update(ui.ctx(), project);
    crate::theme::note(
        ui,
        "A Snapshot is a saved shot: the camera, the lighting, the picture's look and \
         which versions of the model and its materials show. Click one to go back to it.",
    );
    let original = project.document().studios.clone();
    let mut list = original.clone();
    let mut apply = None;
    let mut remove = None;
    let mut swap = None;

    if ui
        .button("+ Snapshot from the current shot")
        .on_hover_text("Captures everything as it is now.")
        .clicked()
    {
        let name = format!("Snapshot {}", list.len() + 1);
        list.push(Studio::capture(&name, project.document()));
    }

    let count = list.len();
    let keys: Vec<u64> = list.iter().map(Studio::picture_key).collect();
    let rendering = thumbnails.job.as_ref().map(|job| job.index);
    let now = Studio::capture("", project.document());
    let mut rows = Vec::with_capacity(count);
    for (index, studio) in list.iter_mut().enumerate() {
        let row = ui.push_id(("studio", index), |ui| {
            ui.separator();
            // The shot it recalls: click it to apply it, drag it to move it.
            let size = egui::vec2(160.0, 90.0);
            match thumbnails.shown.get(&keys[index]) {
                Some(shown) => {
                    let shape = shown.texture.size_vec2();
                    let size = egui::vec2(size.x, (size.x * shape.y / shape.x.max(1.0)).min(size.x));
                    let picture = ui
                        .add(egui::Image::new(&shown.texture).fit_to_exact_size(size).rounding(6.0).sense(egui::Sense::click_and_drag()))
                        .on_hover_text("Click to apply this shot; drag to move it in the list");
                    reorder.grab(&picture, index);
                    if picture.clicked() {
                        apply = Some(index);
                    }
                    // Out of date and being rendered again: a small spinner
                    // in its corner, the old picture shown meanwhile.
                    // Painted, not laid out, so the row keeps its shape.
                    if rendering == Some(index) {
                        let corner = egui::Rect::from_min_size(picture.rect.right_top() + egui::vec2(-20.0, 4.0), egui::vec2(16.0, 16.0));
                        ui.painter().circle_filled(corner.center(), 10.0, crate::theme::p().panel.gamma_multiply(0.85));
                        egui::Spinner::new().size(12.0).paint_at(ui, corner);
                        ui.ctx().request_repaint();
                    }
                }
                None => {
                    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::drag());
                    reorder.grab(&response, index);
                    ui.painter().rect_filled(rect, 6.0, crate::theme::p().control);
                    if !thumbnails.failed.contains(&keys[index]) {
                        ui.put(rect, egui::Spinner::new());
                    }
                }
            }
            ui.horizontal(|ui| {
                // The grip, to drag it up or down the list.
                let (grip, response) = ui.allocate_exact_size(egui::vec2(14.0, 20.0), egui::Sense::drag());
                let lit = response.hovered() || reorder.dragging == Some(index);
                let p = crate::theme::p();
                crate::icons::paint(ui.painter(), grip.shrink2(egui::vec2(1.0, 3.0)), crate::icons::Icon::Grip, if lit { p.text } else { p.text_faint });
                reorder.grab(&response, index);
                response.on_hover_text("Drag up or down the list");
                if ui.button("▶ Apply").on_hover_text("Make the scene this shot. One undo step.").clicked() {
                    apply = Some(index);
                }
                ui.add(egui::TextEdit::singleline(&mut studio.name).desired_width(120.0));
                if ui.add_enabled(index > 0, egui::Button::new("Up").small()).on_hover_text("Move up the list").clicked() {
                    swap = Some((index, index - 1));
                }
                if ui.add_enabled(index + 1 < count, egui::Button::new("Down").small()).on_hover_text("Move down the list").clicked() {
                    swap = Some((index, index + 1));
                }
                if ui.small_button("×").on_hover_text("Delete").clicked() {
                    remove = Some(index);
                }
            });
            // What it holds. Ticking takes the part from the scene as it is
            // now; unticking leaves that part alone when the Studio is
            // applied.
            ui.horizontal_wrapped(|ui| {
                ui.label("Holds:");
                let mut camera = studio.camera.is_some();
                if ui.add(crate::field::Checkbox::new(&mut camera, "Camera")).changed() {
                    studio.camera = camera.then(|| now.camera.clone()).flatten();
                }
                let mut environment = studio.environment.is_some();
                if ui.add(crate::field::Checkbox::new(&mut environment, "Environment")).changed() {
                    studio.environment = environment.then(|| now.environment.clone()).flatten();
                }
                let mut style = studio.image_style.is_some();
                if ui.add(crate::field::Checkbox::new(&mut style, "Image")).changed() {
                    studio.image_style = style.then_some(now.image_style).flatten();
                }
                let mut set = studio.model_set.is_some();
                if ui.add(crate::field::Checkbox::new(&mut set, "Model set")).changed() {
                    studio.model_set = set.then_some(now.model_set).flatten();
                }
                let mut variants = !studio.variants.is_empty();
                if ui.add(crate::field::Checkbox::new(&mut variants, "Variants")).changed() {
                    studio.variants = if variants { now.variants.clone() } else { Vec::new() };
                }
                let mut lights = !studio.lights.is_empty();
                if ui
                    .add(crate::field::Checkbox::new(&mut lights, "Lights"))
                    .on_hover_text("Which lights are on: a view can switch off what another needs, as a top view does its side strips.")
                    .changed()
                {
                    studio.lights = if lights { now.lights.clone() } else { Vec::new() };
                }
                let mut turn = studio.product_yaw.is_some();
                if ui
                    .add(crate::field::Checkbox::new(&mut turn, "Product turn"))
                    .on_hover_text("How the product is turned on its vertical axis: one set of lights and cameras, the product turned for each view, as on a turntable.")
                    .changed()
                {
                    studio.product_yaw = turn.then_some(now.product_yaw).flatten();
                }
            });
            if let Some(yaw) = studio.product_yaw.filter(|y| y.abs() > 1e-4) {
                ui.label(egui::RichText::new(format!("Turns the product {:+.0}°", yaw.to_degrees())).small().color(crate::theme::dim()));
            }
            if ui
                .small_button("Update from the current shot")
                .on_hover_text("Replace what it holds with the scene as it is now.")
                .clicked()
            {
                if studio.camera.is_some() {
                    studio.camera = now.camera.clone();
                }
                if studio.environment.is_some() {
                    studio.environment = now.environment.clone();
                }
                if studio.image_style.is_some() {
                    studio.image_style = now.image_style;
                }
                if studio.model_set.is_some() {
                    studio.model_set = now.model_set;
                }
                if !studio.lights.is_empty() {
                    studio.lights = now.lights.clone();
                }
                if studio.product_yaw.is_some() {
                    studio.product_yaw = now.product_yaw;
                }
                if !studio.variants.is_empty() {
                    studio.variants = now.variants.clone();
                }
            }
        });
        rows.push(row.response.rect);
    }
    reorder.rows = rows;
    let dropped = reorder.show(ui, &list, |index| thumbnails.shown.get(&keys[index]).map(|s| s.texture.clone()));
    if let Some(index) = remove {
        list.remove(index);
    }
    if let Some((a, b)) = swap {
        list.swap(a, b);
    }
    if let Some((from, slot)) = dropped {
        if let Some(reordered) = moved(list.clone(), from, slot) {
            list = reordered;
        }
    }
    if list != original {
        let result = project.apply(Command::SetStudios { studios: list });
        report(project, result);
    }
    if let Some(index) = apply {
        let moves_camera = project.document().studios.get(index).is_some_and(|s| s.camera.is_some());
        let result = project.apply(Command::ApplyStudio { index });
        let applied = result.is_ok();
        report(project, result);
        if applied {
            project.status = format!("Applied {}", project.document().studios[index].name);
            return moves_camera;
        }
    }
    false
}

// ---- Model sets -----------------------------------------------------------------

fn model_sets(ui: &mut egui::Ui, project: &mut Project) {
    crate::theme::note(ui,
        "Versions of the model in one project: one set is shown at a time, and \
         an always-visible set — a backdrop, a stand — shows with every one.",
    );
    let original = project.document().model_sets.clone();
    let mut sets: ModelSets = original.clone();
    let selection: Vec<NodeId> = project.selection.nodes().collect();

    ui.horizontal(|ui| {
        if ui
            .add_enabled(!selection.is_empty(), egui::Button::new("+ Set from selection"))
            .on_hover_text("A new set of the selected nodes, shown straight away.")
            .clicked()
        {
            sets.sets.push(ModelSet {
                name: format!("Model Set {}", sets.sets.len() + 1),
                nodes: selection.clone(),
                always_visible: false,
            });
            sets.active = Some(sets.sets.len() - 1);
        }
        if ui
            .add(egui::SelectableLabel::new(sets.active.is_none(), "Show all"))
            .on_hover_text("Every set at once.")
            .clicked()
        {
            sets.active = None;
        }
    });

    let mut remove = None;
    for index in 0..sets.sets.len() {
        ui.push_id(("set", index), |ui| {
            ui.horizontal(|ui| {
                let shown = sets.active == Some(index);
                if ui.radio(shown, "").on_hover_text("Show this set").clicked() {
                    sets.active = Some(index);
                }
                let set = &mut sets.sets[index];
                ui.add(egui::TextEdit::singleline(&mut set.name).desired_width(110.0));
                ui.add(crate::field::Checkbox::new(&mut set.always_visible, "Always"))
                    .on_hover_text("Always visible: shown whichever set is active.");
                crate::theme::note(ui, format!("{} nodes", set.nodes.len()));
                if ui
                    .add_enabled(!selection.is_empty(), egui::Button::new("Use selection").small())
                    .on_hover_text("Make the selection this set's members.")
                    .clicked()
                {
                    set.nodes = selection.clone();
                }
                if ui.small_button("×").on_hover_text("Delete the set, not its nodes").clicked() {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        sets.sets.remove(index);
        sets.active = match sets.active {
            Some(a) if a == index => None,
            Some(a) if a > index => Some(a - 1),
            other => other,
        };
    }
    if sets != original {
        let result = project.apply(Command::SetModelSets {
            model_sets: Box::new(sets),
        });
        report(project, result);
    }
}

// ---- Material variants -------------------------------------------------------------

fn variants(ui: &mut egui::Ui, project: &mut Project) {
    crate::theme::note(ui,
        "Colourways: several materials for the same parts, one in use. Switching \
         is one click and one undo step, and a Snapshot can remember the choice.",
    );
    let original = project.document().multi_materials.clone();
    let mut list: Vec<MultiMaterial> = original.clone();
    let parts = selected_parts(project);
    let mut duplicate: Option<usize> = None;

    if ui
        .add_enabled(!parts.is_empty(), egui::Button::new("+ Variants from selection"))
        .on_hover_text(
            "The selected parts' material becomes the first variant; add more \
             with Duplicate, then edit each in the Material tab.",
        )
        .clicked()
    {
        let document = project.document();
        // The active part's material, or the first selected part's.
        let first = project
            .selection
            .active()
            .and_then(|n| document.tree.get(n))
            .and_then(|n| n.material)
            .or_else(|| parts.iter().find_map(|&p| document.tree.get(p).and_then(|n| n.material)));
        match first {
            Some(material) => {
                let members: Vec<NodeId> = parts
                    .iter()
                    .copied()
                    .filter(|&p| document.tree.get(p).and_then(|n| n.material) == Some(material))
                    .collect();
                let skipped = parts.len() - members.len();
                let id = list.iter().map(|m| m.id).max().map_or(1, |m| m + 1);
                let name = document.materials.get(material).map_or("Variants".into(), |d| d.name.clone());
                list.push(MultiMaterial {
                    id,
                    name,
                    variants: vec![material],
                    active: 0,
                    parts: members,
                });
                if skipped > 0 {
                    project.status = format!("{skipped} selected parts wear another material and were left out");
                }
            }
            None => project.status = "The selected parts have no material to start from".into(),
        }
    }

    let mut remove = None;
    for index in 0..list.len() {
        ui.push_id(("variants", index), |ui| {
            ui.separator();
            let document = project.document();
            let multi = &mut list[index];
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut multi.name).desired_width(120.0));
                crate::theme::note(ui, format!("{} parts", multi.parts.len()));
                if ui.small_button("×").on_hover_text("Delete the variants; parts keep the one in use").clicked() {
                    remove = Some(index);
                }
            });
            let mut take_out = None;
            for (v, &material) in multi.variants.iter().enumerate() {
                ui.horizontal(|ui| {
                    let name = document.materials.get(material).map_or("(missing)".to_string(), |d| d.name.clone());
                    if ui.radio(multi.active == v, name).on_hover_text("Use this variant").clicked() {
                        multi.active = v;
                    }
                    if multi.variants.len() > 1 && ui.small_button("−").on_hover_text("Remove this variant").clicked() {
                        take_out = Some(v);
                    }
                });
            }
            if let Some(v) = take_out {
                multi.variants.remove(v);
                if multi.active >= multi.variants.len() || multi.active > v {
                    multi.active = multi.active.saturating_sub(1);
                }
            }
            ui.horizontal(|ui| {
                if ui
                    .small_button("Duplicate")
                    .on_hover_text("A copy of the variant in use, made the active one, to edit in the Material tab.")
                    .clicked()
                {
                    duplicate = Some(index);
                }
                // Any material already in the project, as another variant.
                egui::ComboBox::from_id_salt(("add variant", index))
                    .selected_text("+ Existing material")
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for (key, definition) in document.materials.iter() {
                            if !multi.variants.contains(&key) && ui.selectable_label(false, &definition.name).clicked() {
                                multi.variants.push(key);
                            }
                        }
                    });
            });
        });
    }
    if let Some(index) = remove {
        list.remove(index);
    }

    if let Some(index) = duplicate {
        duplicate_variant(project, list, index);
        return;
    }
    if list != original {
        let result = project.apply(Command::SetMultiMaterials { multi_materials: list });
        report(project, result);
    }
}

/// Copy the active variant into a new material and make it the active one.
///
/// Unlinking the first member part is what makes the copy — it is the
/// command that already knows how to duplicate a material properly, graph
/// and textures and all — and the variant switch that follows puts the copy
/// on every member. Both in one undo step.
fn duplicate_variant(project: &mut Project, mut list: Vec<MultiMaterial>, index: usize) {
    let Some(&first) = list[index].parts.first() else {
        project.status = "These variants have no parts to copy from".into();
        return;
    };
    project.begin_group();
    // Whatever else was edited in the list this frame goes in first.
    let mut result = project.apply(Command::SetMultiMaterials {
        multi_materials: list.clone(),
    });
    if result.is_ok() {
        result = project.apply(Command::UnlinkMaterial { node: first });
    }
    if result.is_ok() {
        let copy = project.document().tree.get(first).and_then(|n| n.material);
        if let Some(copy) = copy {
            let multi = &mut list[index];
            multi.variants.push(copy);
            multi.active = multi.variants.len() - 1;
            result = project.apply(Command::SetMultiMaterials { multi_materials: list });
        }
    }
    project.end_group();
    report(project, result);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_studios(count: usize) -> Project {
        let mut project = Project::with_document(
            tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2),
            std::sync::Arc::new(tequila_scene::demo::studio_environment()),
        );
        let mut studios = Vec::new();
        for index in 0..count {
            let mut studio = Studio::capture(&format!("View {index}"), project.document());
            // Each from its own side, so each has a picture of its own.
            studio.product_yaw = Some(index as f32 * 0.7);
            studios.push(studio);
        }
        project.apply(Command::SetStudios { studios }).expect("studios");
        project
    }

    fn names(project: &Project) -> Vec<String> {
        project.document().studios.iter().map(|s| s.name.clone()).collect()
    }

    /// Where a Studio lands: before the first row whose middle is below the
    /// pointer; and moving it there, or nowhere when that is its own place.
    #[test]
    fn a_dragged_studio_lands_between_the_rows_it_is_dropped_between() {
        let rows: Vec<egui::Rect> = (0..3).map(|i| egui::Rect::from_min_size(egui::pos2(0.0, i as f32 * 100.0), egui::vec2(200.0, 100.0))).collect();
        assert_eq!(drop_slot(&rows, -10.0), 0);
        assert_eq!(drop_slot(&rows, 49.0), 0);
        assert_eq!(drop_slot(&rows, 51.0), 1);
        assert_eq!(drop_slot(&rows, 260.0), 3);
        assert_eq!(moved(vec!['a', 'b', 'c'], 0, 3), Some(vec!['b', 'c', 'a']));
        assert_eq!(moved(vec!['a', 'b', 'c'], 2, 0), Some(vec!['c', 'a', 'b']));
        assert_eq!(moved(vec!['a', 'b', 'c'], 0, 2), Some(vec!['b', 'a', 'c']));
        assert_eq!(moved(vec!['a', 'b', 'c'], 1, 1), None, "dropped where it was");
        assert_eq!(moved(vec!['a', 'b', 'c'], 1, 2), None, "dropped just below itself");
        assert_eq!(moved(vec!['a', 'b', 'c'], 3, 0), None, "no such Studio");
    }

    struct Harness {
        ctx: egui::Context,
        tab: StudioTab,
        project: Project,
        time: f64,
    }

    impl Harness {
        fn new(studios: usize) -> Harness {
            let mut h = Harness { ctx: egui::Context::default(), tab: StudioTab::default(), project: project_with_studios(studios), time: 0.0 };
            h.frame(Vec::new());
            h.frame(Vec::new());
            h
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 2400.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (tab, project) = (&mut self.tab, &mut self.project);
            let _ = self.ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| tab.show(ui, project));
            });
        }

        fn press(&mut self, at: egui::Pos2, pressed: bool) {
            self.frame(vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
            ]);
        }

        /// A point on Studio `index`'s picture.
        fn picture(&self, index: usize) -> egui::Pos2 {
            self.tab.reorder.rows[index].left_top() + egui::vec2(60.0, 40.0)
        }

        fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) {
            self.press(from, true);
            for step in 1..=10 {
                self.frame(vec![egui::Event::PointerMoved(from + (to - from) * (step as f32 / 10.0))]);
            }
        }
    }

    /// A Studio dragged by its picture below the last lands there, as one
    /// undo step; let go where it started, nothing changes; and Escape puts
    /// it back.
    #[test]
    fn a_studio_dragged_by_its_picture_moves_in_the_list_as_one_undo_step() {
        let mut h = Harness::new(3);
        assert_eq!(h.tab.reorder.rows.len(), 3);
        let before = names(&h.project);
        let depth = h.project.bus().depth();

        let below = egui::pos2(h.picture(0).x, h.tab.reorder.rows[2].bottom() - 4.0);
        h.drag(h.picture(0), below);
        assert_eq!(h.tab.reorder.dragging, Some(0), "the picture did not start a drag");
        assert_eq!(drop_slot(&h.tab.reorder.rows, below.y), 3);
        assert_eq!(names(&h.project), before, "it moved before it was let go");
        h.press(below, false);
        h.frame(Vec::new());
        assert_eq!(names(&h.project), ["View 1", "View 2", "View 0"]);
        assert_eq!(h.project.bus().depth(), depth + 1, "the move is one step");
        h.project.undo();
        assert_eq!(names(&h.project), before, "one undo puts it back");

        // Let go where it started: nothing.
        h.frame(Vec::new());
        let start = h.picture(1);
        h.drag(start, start + egui::vec2(0.0, 12.0));
        h.press(start + egui::vec2(0.0, 12.0), false);
        h.frame(Vec::new());
        assert_eq!(names(&h.project), before);

        // Escape part-way: nothing.
        let to = egui::pos2(h.picture(2).x, h.tab.reorder.rows[0].top() + 5.0);
        h.drag(h.picture(2), to);
        assert_eq!(h.tab.reorder.dragging, Some(2));
        h.frame(vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        assert_eq!(h.tab.reorder.dragging, None, "Escape did not let it go");
        h.press(to, false);
        h.frame(Vec::new());
        assert_eq!(names(&h.project), before, "Escape still moved it");
    }

    /// A picture out of date keeps its row's shape while it renders again:
    /// the spinner in its corner is painted over it, not laid out, which
    /// pulled the row below up over the picture.
    #[test]
    fn a_picture_being_rendered_again_keeps_its_row_in_shape() {
        let mut h = Harness::new(2);
        let started = std::time::Instant::now();
        while h.tab.thumbnails.current(&h.project) < 2 {
            assert!(started.elapsed().as_secs() < 300, "the pictures never came");
            std::thread::sleep(std::time::Duration::from_millis(20));
            h.frame(Vec::new());
        }
        h.frame(Vec::new());
        let shown: Vec<f32> = h.tab.reorder.rows.iter().map(|r| r.height()).collect();
        // The tab sees the edit when it next draws, and the scene has been
        // still from then on.
        h.project.apply(Command::TurnProduct { yaw: 0.3 }).unwrap();
        h.frame(Vec::new());
        std::thread::sleep(STILL);
        h.frame(Vec::new());
        h.frame(Vec::new());
        assert!(h.tab.thumbnails.job.as_ref().is_some_and(|job| job.index == 0), "the first picture is not being rendered again");
        let rendering: Vec<f32> = h.tab.reorder.rows.iter().map(|r| r.height()).collect();
        assert_eq!(rendering, shown, "a row changed its shape while its picture rendered again");
    }

    /// Each Studio's picture renders on another thread while the UI keeps
    /// drawing; it is kept with the project, saved with it and shown at
    /// once when the project is opened again. A rename keeps it; a change
    /// to the scene puts it out of date, and it is rendered again once the
    /// scene has been still a moment. Applying a Studio leaves out of date
    /// only what that Studio leaves alone - here, nothing.
    #[test]
    fn studios_pictures_render_in_the_background_and_are_saved_with_the_project() {
        let ctx = egui::Context::default();
        let mut project = project_with_studios(2);
        let mut thumbnails = Thumbnails::default();
        let started = std::time::Instant::now();
        let mut longest = std::time::Duration::ZERO;
        let mut frames = 0;
        while thumbnails.current(&project) < 2 {
            let t = std::time::Instant::now();
            thumbnails.update(&ctx, &mut project);
            longest = longest.max(t.elapsed());
            frames += 1;
            assert!(started.elapsed().as_secs() < 300, "the pictures never came");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let total = started.elapsed();
        eprintln!("two Studio pictures in {total:.2?}, {frames} frames, the longest {longest:.2?} on the UI thread");
        assert!(longest < total / 4, "the UI thread waited on the render: {longest:?} of {total:?}");
        assert_eq!(project.document().studio_pictures.len(), 2, "the pictures were not kept with the project");

        // Renamed: the same picture, nothing rendered.
        let mut studios = project.document().studios.clone();
        studios[0].name = "Hero".into();
        project.apply(Command::SetStudios { studios }).unwrap();
        thumbnails.update(&ctx, &mut project);
        assert_eq!(thumbnails.current(&project), 2, "a rename put the picture out of date");
        assert!(thumbnails.job.is_none());

        // Applying a Studio that holds everything leaves the other's
        // picture as it was.
        project.apply(Command::ApplyStudio { index: 1 }).unwrap();
        thumbnails.update(&ctx, &mut project);
        assert_eq!(thumbnails.current(&project), 2, "applying a Studio put a picture out of date");

        // Saved, and opened again: shown at once.
        let directory = std::env::temp_dir().join(format!("tequila-studio-pictures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("studios.tequila");
        project.save(&path).unwrap();
        let mut reopened = Project::with_document(tequila_scene::SceneDocument::new(), std::sync::Arc::new(tequila_scene::demo::studio_environment()));
        reopened.open(&path).unwrap();
        let mut fresh = Thumbnails::default();
        fresh.update(&ctx, &mut reopened);
        assert_eq!(fresh.current(&reopened), 2, "the saved pictures were not shown at once");
        assert!(fresh.job.is_none(), "a saved picture was rendered again");
        let _ = std::fs::remove_dir_all(&directory);

        // An edit to the scene: out of date, but not rendered again until
        // the scene has been still a moment.
        project.apply(Command::TurnProduct { yaw: 0.3 }).unwrap();
        thumbnails.update(&ctx, &mut project);
        assert_eq!(thumbnails.current(&project), 0, "the pictures were not put out of date");
        assert!(thumbnails.job.is_none(), "rendered again at once, not once the scene was still");
        std::thread::sleep(STILL);
        thumbnails.update(&ctx, &mut project);
        assert!(thumbnails.job.is_some(), "never rendered again");
    }
}
