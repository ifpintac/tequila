//! Network rendering, in the Render window: the render nodes, the farm's
//! token, and the renders sent to them (`tequila_network`).
//!
//! A node is any machine running `tequila-cli --worker` with the same
//! token. The shot as it is set up, or every configuration, goes out as one
//! job per picture; each node takes the next when it is free, and the
//! pictures land in the render folder as they would from here. The nodes'
//! addresses are remembered (`~/.tequila/farm.txt`); the token is not, and
//! comes from `TEQUILA_FARM_TOKEN` when that is set.

use crate::project::Project;
use crate::render_window::{image_format, open_file, output_directory, project_name};
use tequila_format::output::{expand_template, first_free, ImageFormat, NameFields};
use tequila_network::{Farm, Job, JobSpec, JobState, WorkerState};
use tequila_scene::render::RenderSettings;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct NetworkPanel {
    token: String,
    addresses: Vec<String>,
    adding: String,
    /// The farm, made with the token it was made with: a new token makes a
    /// new farm.
    farm: Option<(Farm, String)>,
    loaded: bool,
    note: Option<String>,
    /// What was sent, by job: its size and samples, for the filmstrip, and
    /// whether it is there yet.
    sent: Vec<(u64, usize, usize, u32, bool)>,
}

impl Default for NetworkPanel {
    fn default() -> Self {
        NetworkPanel {
            token: std::env::var("TEQUILA_FARM_TOKEN").unwrap_or_default(),
            addresses: Vec::new(),
            adding: String::new(),
            farm: None,
            loaded: false,
            note: None,
            sent: Vec::new(),
        }
    }
}

fn remembered() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join(".tequila").join("farm.txt")
}

impl NetworkPanel {
    /// Whether there are nodes to show, so the section opens by itself.
    pub fn has_nodes(&mut self) -> bool {
        self.load();
        !self.addresses.is_empty()
    }

    /// Whether renders are out, so the window keeps redrawing to show them.
    pub fn busy(&self) -> bool {
        self.farm.as_ref().is_some_and(|(farm, _)| !farm.finished())
    }

    /// Renders the nodes have finished since last asked, for the Render
    /// window's filmstrip, where local ones go.
    pub fn finished(&mut self) -> Vec<crate::render_window::GalleryEntry> {
        let Some((farm, _)) = &self.farm else { return Vec::new() };
        let mut out = Vec::new();
        for job in farm.jobs() {
            let JobState::Done { seconds, .. } = job.state else { continue };
            let Some(entry) = self.sent.iter_mut().find(|s| s.0 == job.id && !s.4) else { continue };
            entry.4 = true;
            out.push(crate::render_window::GalleryEntry {
                name: job.name.clone(),
                path: job.output.to_string_lossy().into_owned(),
                width: entry.1,
                height: entry.2,
                samples: entry.3,
                seconds,
                when: crate::clock::stamp(),
                texture: None,
            });
        }
        out
    }

    fn load(&mut self) {
        if !self.loaded {
            self.loaded = true;
            if let Ok(text) = std::fs::read_to_string(remembered()) {
                self.addresses = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string).collect();
            }
        }
    }

    fn remember(&self) {
        let path = remembered();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, self.addresses.iter().map(|a| format!("{a}\n")).collect::<String>());
    }

    /// The farm for the current token and nodes, made when first needed.
    fn farm(&mut self) -> Option<&Farm> {
        if self.token.len() < 8 {
            return None;
        }
        if self.farm.as_ref().is_some_and(|(_, token)| *token != self.token) {
            self.farm = None;
        }
        if self.farm.is_none() {
            let farm = Farm::new(self.token.clone());
            for address in &self.addresses {
                farm.add_worker(address);
            }
            self.farm = Some((farm, self.token.clone()));
        }
        self.farm.as_ref().map(|(farm, _)| farm)
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, settings: &RenderSettings, project: &mut Project) {
        self.load();
        let dim = crate::theme::dim();
        ui.label(egui::RichText::new("Render on machines running tequila-cli --worker with the same token. Projects travel unencrypted: use a network you trust.").color(dim).small());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Token");
            ui.add(egui::TextEdit::singleline(&mut self.token).password(true).hint_text("8 characters or more").desired_width(f32::INFINITY));
        });
        if !self.token.is_empty() && self.token.len() < 8 {
            ui.label(egui::RichText::new("The token needs at least 8 characters.").color(crate::theme::p().danger).small());
        }

        // The nodes.
        ui.add_space(6.0);
        let statuses = self.farm().map(|f| f.workers()).unwrap_or_default();
        let mut remove = None;
        for address in &self.addresses {
            let status = statuses.iter().find(|s| s.address == tequila_network::farm::address(address));
            let palette = crate::theme::p();
            let (colour, text) = match status.map(|s| &s.state) {
                None => (palette.text_faint, "waiting for the token".to_string()),
                Some(WorkerState::Connecting) => (palette.busy, "connecting".to_string()),
                Some(WorkerState::Idle) => (palette.success, "ready".to_string()),
                Some(WorkerState::Rendering { seconds, .. }) => (palette.accent, format!("rendering, {}", crate::render_window::duration(*seconds))),
                Some(WorkerState::Offline(why)) => (palette.danger, why.clone()),
            };
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.0, colour);
                let name = status.map(|s| s.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| address.clone());
                let cores = status.map(|s| s.cores).filter(|c| *c > 0).map(|c| format!(" · {c} cores")).unwrap_or_default();
                ui.label(egui::RichText::new(format!("{name}{cores}")).strong()).on_hover_text(address);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Remove").on_hover_text("Stop using this node; what it is rendering finishes first").clicked() {
                        remove = Some(address.clone());
                    }
                    ui.label(egui::RichText::new(crate::render_window::clip(&text, 34)).color(dim).small()).on_hover_text(&text);
                });
            });
        }
        if let Some(address) = remove {
            self.addresses.retain(|a| *a != address);
            if let Some((farm, _)) = &self.farm {
                farm.remove_worker(&address);
            }
            self.remember();
        }
        ui.horizontal(|ui| {
            let field = ui.add(egui::TextEdit::singleline(&mut self.adding).hint_text("host or host:port").desired_width(ui.available_width() - 60.0));
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Add").clicked() || entered) && !self.adding.trim().is_empty() {
                let address = self.adding.trim().to_string();
                if !self.addresses.contains(&address) {
                    self.addresses.push(address.clone());
                    if let Some((farm, _)) = &self.farm {
                        farm.add_worker(&address);
                    }
                    self.remember();
                }
                self.adding.clear();
            }
        });

        // Sending.
        ui.add_space(6.0);
        let ready = self.token.len() >= 8 && !self.addresses.is_empty();
        let configurations = !project.document().configurator.is_empty();
        ui.horizontal(|ui| {
            if ui.add_enabled(ready, egui::Button::new("Render shot")).on_hover_text("The shot as it is set up, on the first free node").clicked() {
                if let Err(why) = self.send(settings, project, false) {
                    self.note = Some(why);
                }
            }
            if configurations && ui.add_enabled(ready, egui::Button::new("All configurations")).on_hover_text("Every valid configuration, shared out among the nodes").clicked() {
                if let Err(why) = self.send(settings, project, true) {
                    self.note = Some(why);
                }
            }
        });
        if !matches!(image_format(settings), ImageFormat::Png(_) | ImageFormat::Jpeg(_)) {
            ui.label(egui::RichText::new("Render nodes write PNG or JPEG; these go out as PNG.").color(dim).small());
        }
        if let Some(note) = &self.note {
            ui.label(egui::RichText::new(note).color(dim).small());
        }

        // The jobs, newest first.
        let jobs = self.farm.as_ref().map(|(farm, _)| farm.jobs()).unwrap_or_default();
        if jobs.is_empty() {
            return;
        }
        ui.add_space(6.0);
        let mut retry = None;
        for job in jobs.iter().rev() {
            let palette = crate::theme::p();
            ui.horizontal(|ui| {
                let (colour, text) = match &job.state {
                    JobState::Queued => (palette.text_faint, "queued".to_string()),
                    JobState::Rendering { worker, seconds } => (palette.accent, format!("{worker}, {}", crate::render_window::duration(*seconds))),
                    JobState::Done { worker, seconds } => (palette.success, format!("{worker}, {}", crate::render_window::duration(*seconds))),
                    JobState::Failed(why) => (palette.danger, why.clone()),
                };
                ui.label(egui::RichText::new(crate::render_window::clip(&job.name, 26))).on_hover_text(job.output.display().to_string());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match &job.state {
                        JobState::Done { .. } => {
                            if ui.small_button("Open").clicked() {
                                open_file(&job.output.to_string_lossy());
                            }
                        }
                        JobState::Failed(_) => {
                            if ui.small_button("Retry").clicked() {
                                retry = Some(job.id);
                            }
                        }
                        _ => {}
                    }
                    ui.label(egui::RichText::new(crate::render_window::clip(&text, 30)).color(colour).small()).on_hover_text(&text);
                });
            });
        }
        if let Some(id) = retry {
            if let Some((farm, _)) = &self.farm {
                farm.retry(id);
            }
        }
        if jobs.iter().any(|j| matches!(j.state, JobState::Done { .. } | JobState::Failed(_))) && ui.small_button("Clear finished").clicked() {
            if let Some((farm, _)) = &self.farm {
                farm.clear_finished();
            }
        }
        if self.busy() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// Send the shot, or every configuration, to the farm: how many renders
    /// went, or why none did.
    pub fn send(&mut self, settings: &RenderSettings, project: &mut Project, every_configuration: bool) -> Result<usize, String> {
        self.load();
        if self.token.len() < 8 {
            return Err("the farm needs its token, 8 characters or more: in the Render window's Network rendering section, or TEQUILA_FARM_TOKEN".into());
        }
        if self.addresses.is_empty() {
            return Err("there are no render nodes: add them in the Render window's Network rendering section".into());
        }
        if every_configuration && project.document().configurator.is_empty() {
            return Err("there is no configurator to render the configurations of".into());
        }
        let document = project.document();
        let base = project.path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
        let bundle = tequila_network::bundle::pack(document, &tequila_format::Preserved::default(), base.as_deref());
        let format = if matches!(image_format(settings), ImageFormat::Jpeg(_)) { "jpg" } else { "png" };
        let folder = output_directory(settings, project.path.as_deref());
        let fields = NameFields {
            project: project_name(&document.meta.name, project.path.as_deref()),
            camera: project.camera().name.clone(),
            width: settings.width,
            height: settings.height,
            samples: settings.max_samples,
            date: crate::today(),
            time: crate::now(),
            ..Default::default()
        };
        let spec = JobSpec {
            width: Some(settings.width),
            height: Some(settings.height),
            samples: Some(settings.max_samples),
            format: format.into(),
            engine: matches!(settings.engine, tequila_scene::render::RenderEngine::CyclesCpu | tequila_scene::render::RenderEngine::CyclesGpu).then(|| "cycles".to_string()),
            alpha: settings.alpha_mode.transparent_background(),
            silhouette: settings.alpha_mode == tequila_scene::render::AlphaMode::Silhouette,
            ..JobSpec::default()
        };
        let mut planned: Vec<(JobSpec, PathBuf)> = Vec::new();
        let mut taken: Vec<PathBuf> = Vec::new();
        // A file name not yet on disk, nor taken by another in this batch.
        let mut place = |name: &str| {
            let mut path = first_free(folder.join(format!("{name}.{format}")));
            let mut n = 2;
            while taken.contains(&path) {
                path = folder.join(format!("{name}_{n}.{format}"));
                n += 1;
            }
            taken.push(path.clone());
            path
        };
        if every_configuration {
            let configurator = &document.configurator;
            let mut template = settings.filename_template.clone();
            if !template.contains("{configuration}") {
                template.push_str(" {configuration}");
            }
            for choices in configurator.combinations() {
                let words = configurator.describe(&choices).join(" - ");
                let name = expand_template(&template, &NameFields { configuration: words.clone(), ..fields.clone() });
                let configuration = choices.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(",");
                planned.push((JobSpec { name: words, configuration: Some(configuration), ..spec.clone() }, place(&name)));
            }
        } else {
            let name = expand_template(&settings.filename_template, &fields);
            planned.push((JobSpec { name: name.clone(), ..spec }, place(&name)));
        }
        let missing = bundle.missing.clone();
        let Some(farm) = self.farm() else { return Err("the farm could not be started".into()) };
        let count = planned.len();
        let mut ids = Vec::new();
        for (spec, output) in planned {
            ids.push(farm.submit(Job { spec, bundle: Arc::clone(&bundle.bytes), output }));
        }
        self.sent.extend(ids.into_iter().map(|id| (id, settings.width, settings.height, settings.max_samples, false)));
        self.note = Some(if missing.is_empty() {
            format!("{count} render{} sent, {:.1} MB each way.", if count == 1 { "" } else { "s" }, bundle.bytes.len() as f64 / 1e6)
        } else {
            format!("{count} sent; {} file{} could not be read and go without: {}", missing.len(), if missing.len() == 1 { "" } else { "s" }, missing.join(", "))
        });
        project.status = format!("Sent {count} render{} to the render nodes", if count == 1 { "" } else { "s" });
        Ok(count)
    }
}
