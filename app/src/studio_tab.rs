//! The Studios tab: saved shots, model sets and material variants
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
    /// What the configurator asked for this frame - to present, or to
    /// render every combination - drained by the application.
    pub action: Option<crate::configurator_panel::Action>,
    /// Scroll the configurator into view on the next frame.
    pub reveal: bool,
    /// Render every Studio, one image each, asked for this frame.
    pub render_all: bool,
}

impl StudioTab {
    /// Returns true when a Studio moved the camera, so the viewport's own
    /// camera has to follow.
    pub fn show(&mut self, ui: &mut egui::Ui, project: &mut Project) -> bool {
        let mut camera_moved = false;
        crate::theme::section(ui, "Studios");
        camera_moved |= studios(ui, project, &mut self.thumbnails);
        let views = project.document().studios.len();
        if views > 1
            && ui
                .button(format!("Render all {views} views"))
                .on_hover_text("Queue every Studio, one image each, named for the view: {view} in the file name.")
                .clicked()
        {
            self.render_all = true;
        }
        ui.add_space(6.0);
        ui.separator();
        crate::theme::section(ui, "Model sets");
        model_sets(ui, project);
        ui.add_space(6.0);
        ui.separator();
        crate::theme::section(ui, "Material variants");
        variants(ui, project);
        ui.add_space(6.0);
        ui.separator();
        crate::theme::section(ui, "Configurator");
        if std::mem::take(&mut self.reveal) {
            ui.scroll_to_cursor(Some(egui::Align::TOP));
        }
        let before = project.document().configurator.current.clone();
        self.action = self.wizard.show(ui, project).or(self.action.take());
        // A configuration with a Studio in it can move the camera.
        let configurator = &project.document().configurator;
        if configurator.current != before
            && configurator.options.iter().any(|o| o.drives == tequila_scene::configurator::Drives::Studio)
        {
            camera_moved = true;
        }
        camera_moved
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
/// thread, one at a time, and kept until the Studio changes or another
/// project opens (`TEQUILA_SPEC.md` §1.12, "auto thumbnails").
#[derive(Default)]
struct Thumbnails {
    shown: std::collections::HashMap<u64, egui::TextureHandle>,
    /// The thumbnail being rendered, by its key.
    job: Option<(u64, std::thread::JoinHandle<Option<egui::ColorImage>>)>,
    /// Ones that would not render, so they are not tried every frame.
    failed: std::collections::HashSet<u64>,
    /// Studios' environments, apart from the scene's.
    environment: tequila_scene::environment::EnvironmentCache,
}

/// A thumbnail's width in pixels; its height follows the render's shape.
const THUMBNAIL_WIDTH: usize = 192;

impl Thumbnails {
    /// What a Studio's thumbnail shows: the Studio, in this project.
    fn key(studio: &Studio, project: &Project) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        format!("{studio:?}").hash(&mut hasher);
        project.document_epoch().hash(&mut hasher);
        hasher.finish()
    }

    /// Take a finished render, and start the next one missing.
    fn update(&mut self, ctx: &egui::Context, project: &mut Project) {
        if self.job.as_ref().is_some_and(|(_, job)| job.is_finished()) {
            if let Some((key, job)) = self.job.take() {
                match job.join().ok().flatten() {
                    Some(image) => {
                        let texture = ctx.load_texture(format!("studio-{key:x}"), image, egui::TextureOptions::LINEAR);
                        self.shown.insert(key, texture);
                    }
                    None => {
                        self.failed.insert(key);
                    }
                }
            }
        }
        if self.job.is_some() {
            return;
        }
        let keys: Vec<u64> = project.document().studios.iter().map(|s| Self::key(s, project)).collect();
        // Thumbnails of Studios gone or changed are let go.
        self.shown.retain(|key, _| keys.contains(key));
        let Some((index, &key)) = keys.iter().enumerate().find(|(_, k)| !self.shown.contains_key(k) && !self.failed.contains(k)) else {
            return;
        };
        let Some((scene, camera, style, config)) = project.studio_scene(index, &mut self.environment) else {
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
        if let Ok(job) = job {
            self.job = Some((key, job));
        }
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

fn studios(ui: &mut egui::Ui, project: &mut Project, thumbnails: &mut Thumbnails) -> bool {
    thumbnails.update(ui.ctx(), project);
    ui.small(
        "A Studio is a saved shot: the camera, the environment, the image style, \
         the model set and the material variants, recalled in one click.",
    );
    let original = project.document().studios.clone();
    let mut list = original.clone();
    let mut apply = None;
    let mut remove = None;
    let mut swap = None;

    if ui
        .button("+ Studio from the current shot")
        .on_hover_text("Captures everything as it is now.")
        .clicked()
    {
        let name = format!("Studio {}", list.len() + 1);
        list.push(Studio::capture(&name, project.document()));
    }

    let count = list.len();
    let keys: Vec<u64> = list.iter().map(|s| Thumbnails::key(s, project)).collect();
    for (index, studio) in list.iter_mut().enumerate() {
        ui.push_id(("studio", index), |ui| {
            ui.separator();
            // The shot it recalls: click it to apply it.
            let size = egui::vec2(160.0, 90.0);
            match thumbnails.shown.get(&keys[index]) {
                Some(texture) => {
                    let shape = texture.size_vec2();
                    let size = egui::vec2(size.x, (size.x * shape.y / shape.x.max(1.0)).min(size.x));
                    let picture = ui
                        .add(egui::Image::new(texture).fit_to_exact_size(size).rounding(6.0).sense(egui::Sense::click()))
                        .on_hover_text("Apply this shot");
                    if picture.clicked() {
                        apply = Some(index);
                    }
                }
                None => {
                    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                    ui.painter().rect_filled(rect, 6.0, crate::theme::p().control);
                    if !thumbnails.failed.contains(&keys[index]) {
                        ui.put(rect, egui::Spinner::new());
                    }
                }
            }
            ui.horizontal(|ui| {
                if ui.button("▶ Apply").on_hover_text("Make the scene this shot. One undo step.").clicked() {
                    apply = Some(index);
                }
                ui.add(egui::TextEdit::singleline(&mut studio.name).desired_width(120.0));
                if ui.small_button("Up").on_hover_text("Move up the list").clicked() && index > 0 {
                    swap = Some((index, index - 1));
                }
                if ui.small_button("Down").on_hover_text("Move down the list").clicked() && index + 1 < count {
                    swap = Some((index, index + 1));
                }
                if ui.small_button("×").on_hover_text("Delete").clicked() {
                    remove = Some(index);
                }
            });
            // What it holds. Ticking takes the part from the scene as it is
            // now; unticking leaves that part alone when the Studio is
            // applied.
            let now = Studio::capture("", project.document());
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
    }
    if let Some(index) = remove {
        list.remove(index);
    }
    if let Some((a, b)) = swap {
        list.swap(a, b);
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
    ui.small(
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
                ui.weak(format!("{} nodes", set.nodes.len()));
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
    ui.small(
        "Colourways: several materials for the same parts, one in use. Switching \
         is one click and one undo step, and a Studio can remember the choice.",
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
                ui.weak(format!("{} parts", multi.parts.len()));
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
