//! `tequila --serve`: the command bus with no window, for scripts.
//!
//! The application answers the scripting channel between frames; this
//! answers the same protocol from a document held here, so a script can open
//! a project, change it, render it and save it on a machine where nobody is
//! looking - and the Python module's `tequila.open()` starts one of these
//! for itself (`docs/scripting-api.md`).
//!
//! Every write is a command through the bus, as in the application: undo,
//! batches as one step, and the journal's validation all hold. The queries
//! that only need the document are the application's own
//! (`tequila_scripting::document`), so a script reads the same answers from
//! either. What needs a window - screenshots, panels, the background render
//! queue - is refused by name.
//!
//! The session announces itself only in the discovery file it is given, so
//! a script's private server never takes clients from the application a
//! person has open. With `--attached` it ends when its standard input does,
//! so a script that dies does not leave it running.

use crate::{each_configuration, render_one, Options};
use tequila_format::json::Value;
use tequila_format::json_write::Object;
use tequila_scene::orbit::OrbitCamera;
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::{Command, CommandBus, SceneDocument};
use tequila_scripting::document::{self, PROTOCOL};
use tequila_scripting::protocol::{Operation, Query, Reply, Request};
use tequila_scripting::server::Channel;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tequila_scripting::document::{HEADLESS_LIMITS as LIMITS, HEADLESS_OPERATIONS as OPERATIONS, HEADLESS_QUERIES as QUERIES};

/// Serve until told to quit, or, attached, until the parent goes.
pub fn run(options: &Options) -> Result<(), String> {
    let mut session = Session::open(options)?;
    let discovery = options
        .discovery
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("tequila-serve-{}.json", std::process::id())));
    let channel = Channel::start_at(&discovery)?;
    eprintln!("tequila: serving on 127.0.0.1:{}, announced in {}", channel.port(), discovery.display());

    let orphaned = Arc::new(AtomicBool::new(false));
    if options.attached {
        let flag = Arc::clone(&orphaned);
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = std::io::Read::read_to_end(&mut std::io::stdin(), &mut sink);
            flag.store(true, Ordering::SeqCst);
        });
    }
    while !session.quit && !orphaned.load(Ordering::SeqCst) {
        channel.drain(|request| session.handle(request));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // The answer to the quit is on its way to the client; let it arrive
    // before the connection goes with the process.
    std::thread::sleep(std::time::Duration::from_millis(200));
    Ok(())
}

/// A document, its history, and what a script has pointed at.
pub struct Session {
    document: SceneDocument,
    preserved: tequila_format::Preserved,
    bus: CommandBus,
    path: Option<PathBuf>,
    selection: Vec<NodeId>,
    camera: OrbitCamera,
    /// Changes so far, for clients that poll for them.
    generation: u64,
    quit: bool,
    /// The command line's own, for renders: the engine, the device.
    options: Options,
}

fn absolute(path: &Path) -> String {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()).display().to_string()
}

impl Session {
    /// A project to open, a model to start from, or an empty scene.
    pub fn open(options: &Options) -> Result<Session, String> {
        let mut session = Session {
            document: SceneDocument::new(),
            preserved: tequila_format::Preserved::default(),
            bus: CommandBus::new(),
            path: None,
            selection: Vec::new(),
            camera: OrbitCamera::framing(tequila_core::math::Vec3::ZERO, 1.0, 1.6),
            generation: 0,
            quit: false,
            options: options.clone(),
        };
        if let Some(input) = &options.input {
            let path = Path::new(input);
            if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("tequila")) {
                let loaded = tequila_format::load(path).map_err(|e| e.to_string())?;
                session.document = loaded.document;
                session.preserved = loaded.preserved;
                session.path = Some(path.to_path_buf());
            } else if let Reply::Failed(error) = session.import(input, None) {
                return Err(error);
            } else {
                session.bus.mark_saved();
            }
        }
        session.camera.adopt(session.document.cameras.active());
        Ok(session)
    }

    pub fn handle(&mut self, request: &Request) -> Reply {
        self.perform(&request.operation)
    }

    /// Keep the orbit in step with the document's camera after something
    /// that may have moved it: a camera command, an undo.
    fn follow_camera(&mut self) {
        let saved = self.document.cameras.active();
        if saved.camera != self.camera.camera || saved.target != self.camera.target {
            self.camera.adopt(saved);
        }
    }

    /// Put the orbit into the document, as its own undo step.
    fn record_camera(&mut self) -> Result<(), String> {
        let saved = self.document.cameras.active();
        if saved.camera == self.camera.camera && saved.target == self.camera.target {
            return Ok(());
        }
        let index = self.document.cameras.active_index();
        let camera = Box::new(self.camera.to_saved(saved.name.clone()));
        self.apply(Command::SetCamera { index, camera })
    }

    fn apply(&mut self, command: Command) -> Result<(), String> {
        self.bus.end_interaction();
        let result = self.bus.apply(&mut self.document, command).map(|_| ()).map_err(|e| e.to_string());
        self.bus.end_interaction();
        if result.is_ok() {
            self.generation += 1;
        }
        result
    }

    fn perform(&mut self, operation: &Operation) -> Reply {
        match operation {
            Operation::Ping => {
                let mut out = Object::new();
                out.set("application", "tequila").set("protocol", PROTOCOL).set("pid", std::process::id() as f64).set("headless", true);
                Reply::Value(out.build())
            }
            Operation::Apply(command) => match self.apply((**command).clone()) {
                Ok(()) => {
                    self.follow_camera();
                    Reply::Done
                }
                Err(error) => Reply::Failed(error),
            },
            Operation::Undo | Operation::Redo => {
                if self.bus.in_group() {
                    return Reply::Failed("undo and redo cannot be part of a batch".into());
                }
                let undoing = matches!(operation, Operation::Undo);
                let step = if undoing { self.bus.undo_step() } else { self.bus.redo_step() };
                let result = if undoing { self.bus.undo(&mut self.document) } else { self.bus.redo(&mut self.document) };
                if let Err(error) = result {
                    return Reply::Failed(error.to_string());
                }
                self.generation += 1;
                self.follow_camera();
                let mut out = Object::new();
                let key = if undoing { "undone" } else { "redone" };
                match step {
                    Some((name, commands)) => out.set(key, name).set("commands", commands as f64),
                    None => out.set(key, Value::Null).set("commands", 0.0),
                };
                Reply::Value(out.build())
            }
            Operation::Select(ids) => {
                let missing: Vec<u64> = ids.iter().copied().filter(|id| self.document.tree.get(NodeId(*id)).is_none()).collect();
                if !missing.is_empty() {
                    return Reply::Failed(format!("no such part: {missing:?}"));
                }
                self.selection = ids.iter().map(|id| NodeId(*id)).collect();
                Reply::Done
            }
            Operation::Query(Query::Ui | Query::Jobs) => Reply::Failed("there is no window, and so no panels or background renders, in a headless session".into()),
            Operation::Query(query) => Reply::Value(self.answer(*query)),
            Operation::Import { path, parent } => self.import(path, *parent),
            Operation::Reimport { node, path, tessellation } => self.reimport(*node, path.as_deref(), tessellation.as_deref()),
            Operation::Save { path } => self.save(path.as_deref()),
            Operation::Frame { all } => self.frame(*all),
            Operation::Look(look) => self.look(look),
            Operation::Render { path, width, height, samples } => self.render(path, *width, *height, *samples),
            Operation::Batch { operations, atomic } => self.batch(operations, *atomic),
            Operation::Library(request) => self.library(request),
            Operation::Material(request) => self.material(request),
            Operation::Quit { save, discard } => self.quit(*save, *discard),
            Operation::Project { action, save, discard } => self.project(action, *save, *discard),
            Operation::Export { format, path, live, photos, columns, rows, tilt, transparent } => {
                self.export(format, path, *live, *photos, (*columns, *rows, *tilt), *transparent)
            }
            Operation::Screenshot { .. } | Operation::Ui(_) | Operation::Queue(_) | Operation::Stop | Operation::TextureSet { .. } | Operation::LiveLink(_) | Operation::Plugin(_) | Operation::StudioPreset(_) => {
                Reply::Failed(format!("`{}` needs the application's window; a headless session cannot do it", operation.summary()))
            }
        }
    }

    /// The material library: presets onto parts through the bus, and the
    /// user's own library added to, exported from or tidied - the same
    /// library the application shows.
    fn library(&mut self, request: &tequila_scripting::protocol::LibraryRequest) -> Reply {
        use tequila_scripting::document::library_request;
        let pack = |path: &std::path::Path, size: f32, name: String, category: String, tags: Vec<String>| {
            tequila_import::texture_set::library_entry(path, size, name, category, tags).map_err(|e| e.to_string())
        };
        let selection: Vec<NodeId> = self.selection.clone();
        match library_request(request, &self.document, &selection, &tequila_format::library::user_directory(), &pack) {
            Ok(outcome) => self.carry_out(outcome),
            Err(error) => Reply::Failed(error),
        }
    }

    /// A material of the caller's own, from a recipe: made, changed, read
    /// back, shown on a ball or kept in the library
    /// (`tequila_scripting::material`).
    fn material(&mut self, request: &tequila_scripting::protocol::MaterialRequest) -> Reply {
        let selection: Vec<NodeId> = self.selection.clone();
        match tequila_scripting::material::material_request(request, &self.document, &selection, &tequila_format::library::user_directory()) {
            Ok(outcome) => self.carry_out(outcome),
            Err(error) => Reply::Failed(error),
        }
    }

    /// Commands as one undo step, all or none, then the answer.
    fn carry_out(&mut self, outcome: tequila_scripting::document::LibraryOutcome) -> Reply {
        use tequila_scripting::document::LibraryOutcome;
        match outcome {
            LibraryOutcome::Commands(commands, value) => {
                self.bus.begin_group();
                for command in commands {
                    if let Err(error) = self.apply(command) {
                        let _ = self.bus.abandon_group(&mut self.document);
                        return Reply::Failed(error);
                    }
                }
                self.bus.end_group();
                Reply::Value(value)
            }
            LibraryOutcome::Done(value) => Reply::Value(value),
        }
    }

    fn answer(&self, query: Query) -> Value {
        match query {
            Query::Library => document::library(&tequila_format::library::user_directory()),
            Query::Api => document::api(OPERATIONS, QUERIES, LIMITS),
            Query::Context => {
                let mut out = Object::new();
                out.set("scene", document::scene(&self.document, self.path.as_deref()))
                    .set("nodes", document::nodes(&self.document))
                    .set("materials", document::materials(&self.document))
                    .set("selection", self.answer(Query::Selection))
                    .set("camera", self.answer(Query::Camera));
                if let Ok(value) = tequila_format::json::parse(tequila_format::scene_description(&self.document).as_bytes()) {
                    out.set("document", value);
                }
                out.set("generation", self.generation as f64);
                out.build()
            }
            Query::Scene => document::scene(&self.document, self.path.as_deref()),
            Query::Nodes => document::nodes(&self.document),
            Query::Materials => document::materials(&self.document),
            Query::Selection => {
                let mut out = Object::new();
                out.set("nodes", Value::Array(self.selection.iter().map(|id| Value::Number(id.0 as f64)).collect()));
                out.build()
            }
            Query::Commands => document::commands(),
            Query::Camera => document::camera(&self.camera, &self.document.cameras.active().name),
            Query::History => {
                let step = |step: Option<(&'static str, usize)>| match step {
                    Some((name, commands)) => {
                        let mut out = Object::new();
                        out.set("command", name).set("commands", commands as f64);
                        out.build()
                    }
                    None => Value::Null,
                };
                let mut out = Object::new();
                out.set("undo", step(self.bus.undo_step()))
                    .set("redo", step(self.bus.redo_step()))
                    .set("unsaved", self.bus.has_unsaved_changes())
                    .set("path", self.path.as_deref().map(absolute).map(Value::String).unwrap_or(Value::Null));
                out.build()
            }
            Query::Ui | Query::Jobs => Value::Null,
        }
    }

    /// Keep a CAD file's own bytes, to divide it again without the file.
    fn keep_source(&mut self, model: &tequila_import::ReadModel) {
        if let Some((id, bytes)) = &model.brep {
            self.document.sources.insert(id.clone(), bytes.clone());
        }
    }

    fn import(&mut self, path: &str, parent: Option<u64>) -> Reply {
        let path = PathBuf::from(path);
        if !path.is_file() {
            return Reply::Failed(format!("{} is not a file", path.display()));
        }
        let model = match tequila_import::read_file(&path) {
            Ok(model) => model,
            Err(error) => return Reply::Failed(error.to_string()),
        };
        self.keep_source(&model);
        let (command, report) = match tequila_import::build_read(&self.document, parent.map(NodeId), model, &tequila_import::ImportOptions::default()) {
            Ok(result) => result,
            Err(error) => return Reply::Failed(error.to_string()),
        };
        if let Err(error) = self.apply(command) {
            return Reply::Failed(error);
        }
        let mut out = Object::new();
        out.set("path", path.display().to_string())
            .set("nodes", report.nodes as f64)
            .set("meshes", report.meshes_kept as f64)
            .set("triangles", report.triangles as f64)
            .set("textures", report.textures as f64)
            .set("unit", format!("{:?}", report.source_unit))
            .set("unit_was_guessed", report.unit_was_guessed)
            .set("warnings", Value::Array(report.warnings.iter().map(|w| Value::String(w.clone())).collect()));
        Reply::Value(out.build())
    }

    fn reimport(&mut self, node: Option<u64>, path: Option<&str>, tessellation: Option<&str>) -> Reply {
        let Some(node) = node.map(NodeId).or_else(|| self.selection.last().copied()) else {
            return Reply::Failed("name a node of the model, or select one".into());
        };
        if !self.document.tree.contains(node) {
            return Reply::Failed(format!("there is no node {}", node.0));
        }
        let roots = tequila_scene::reimport::import_roots(&self.document, node);
        let quality = match tessellation.map(|t| (t, tequila_import::step::Tessellation::from_label(t))) {
            None => None,
            Some((_, Some(quality))) => Some(quality),
            Some((name, None)) => return Reply::Failed(format!("`{name}` is not a tessellation: draft, medium, high or ultra")),
        };
        let kept = roots
            .first()
            .and_then(|r| self.document.tree.get(*r))
            .and_then(|n| n.metadata.get(tequila_scene::document::BREP))
            .and_then(|id| self.document.sources.get(id))
            .cloned();
        let recorded = tequila_scene::reimport::source_file(&self.document, &roots).map(PathBuf::from);
        let read = match (quality, path, kept) {
            (Some(quality), None, Some(bytes)) => {
                let name = recorded.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Model".into());
                tequila_import::read_cad(&bytes, &name, recorded.clone(), quality)
            }
            _ => {
                let file = match path.map(PathBuf::from).or(recorded) {
                    Some(file) => file,
                    None => return Reply::Failed("this model records no file; give a path".into()),
                };
                tequila_import::read_file_with(&file, quality.unwrap_or_default())
            }
        };
        let model = match read {
            Ok(model) => model,
            Err(error) => return Reply::Failed(error.to_string()),
        };
        self.keep_source(&model);
        let parent = self.document.tree.get(roots[0]).and_then(|n| n.parent);
        let options = tequila_import::ImportOptions { place_on_ground: false, ..Default::default() };
        let (command, _) = match tequila_import::build_read(&self.document, parent, model, &options) {
            Ok(result) => result,
            Err(error) => return Reply::Failed(error.to_string()),
        };
        let Command::Import { payload } = command else { return Reply::Failed("the file did not read as a model".into()) };
        let (update, report) = tequila_scene::reimport::plan(&self.document, &roots, *payload);
        if let Err(error) = self.apply(update) {
            return Reply::Failed(error);
        }
        let mut out = Object::new();
        out.set("matched", report.matched as f64)
            .set("added", Value::Array(report.added.iter().map(|n| Value::String(n.clone())).collect()))
            .set(
                "removed",
                Value::Array(
                    report
                        .removed
                        .iter()
                        .map(|(id, name)| {
                            let mut entry = Object::new();
                            entry.set("id", id.0 as f64).set("name", name.clone());
                            entry.build()
                        })
                        .collect(),
                ),
            );
        Reply::Value(out.build())
    }

    fn save(&mut self, path: Option<&str>) -> Reply {
        let Some(target) = path.map(PathBuf::from).or_else(|| self.path.clone()) else {
            return Reply::Failed("this project has never been saved, so a path is needed".into());
        };
        match tequila_format::save(&target, &self.document, &self.preserved) {
            Ok(()) => {
                self.bus.mark_saved();
                self.path = Some(target.clone());
                let mut out = Object::new();
                out.set("path", absolute(&target));
                Reply::Value(out.build())
            }
            Err(error) => Reply::Failed(error.to_string()),
        }
    }

    fn aspect(&self) -> f32 {
        let render = &self.document.render;
        render.width.max(1) as f32 / render.height.max(1) as f32
    }

    /// The application's Frame: the whole scene or the selection, keeping
    /// the direction the camera looks from.
    fn frame(&mut self, all: bool) -> Reply {
        use tequila_core::math::Aabb;
        let bounds = if all || self.selection.is_empty() {
            tequila_scene::snapshot::visible_bounds(&self.document)
        } else {
            let mut bounds = Aabb::EMPTY;
            for &node in &self.selection {
                for id in self.document.tree.descendants_and_self(node) {
                    let Some(&NodeKind::Part { mesh }) = self.document.tree.get(id).map(|n| &n.kind) else { continue };
                    if let Some(source) = self.document.geometry.get(mesh) {
                        bounds.extend(&source.bounds().transformed(&self.document.tree.world_transform(id)));
                    }
                }
            }
            bounds
        };
        if bounds.is_empty() {
            return Reply::Failed("nothing to frame".into());
        }
        let radius = (bounds.diagonal().length() * 0.5).max(1e-3);
        let framed = OrbitCamera::framing(bounds.center(), radius, self.aspect());
        self.camera.frame_at(framed.target, framed.distance);
        match self.record_camera() {
            Ok(()) => Reply::Value(self.answer(Query::Camera)),
            Err(error) => Reply::Failed(error),
        }
    }

    /// The application's `look`, on the session's camera.
    fn look(&mut self, look: &tequila_scripting::protocol::Look) -> Reply {
        use tequila_core::math::Vec3;
        let to_vec = |v: [f32; 3]| Vec3 { x: v[0], y: v[1], z: v[2] };
        let mut orbit = self.camera;
        if let Some(name) = &look.view {
            match OrbitCamera::view_named(name) {
                Some(view) => orbit.set_standard_view(view),
                None => return Reply::Failed(format!("`{name}` is not a view; try front, back, left, right, top, bottom or iso")),
            }
        }
        let target = look.at.map(to_vec).unwrap_or(orbit.target);
        if let Some(from) = look.from {
            let eye = to_vec(from);
            if (eye - target).length() < 1e-6 {
                return Reply::Failed("the camera cannot look at the point it is standing on".into());
            }
            if look.yaw.is_some() || look.pitch.is_some() || look.distance.is_some() {
                return Reply::Failed("give either `from`, or yaw/pitch/distance, not both".into());
            }
            orbit.place_from(eye, target);
        } else if look.at.is_some() || look.yaw.is_some() || look.pitch.is_some() || look.distance.is_some() {
            if look.distance.is_some_and(|d| d <= 0.0) {
                return Reply::Failed("distance must be more than zero".into());
            }
            orbit.place(
                target,
                look.yaw.map(f32::to_radians).unwrap_or(orbit.yaw),
                look.pitch.map(f32::to_radians).unwrap_or(orbit.pitch),
                look.distance.unwrap_or(orbit.distance),
            );
        }
        if let Some(focal) = look.focal_length {
            if !(1.0..=2000.0).contains(&focal) {
                return Reply::Failed("focal_length is in millimetres, between 1 and 2000".into());
            }
            match &mut orbit.camera.projection {
                tequila_render_core::camera::Projection::Perspective { focal_length_mm } | tequila_render_core::camera::Projection::Shift { focal_length_mm, .. } => {
                    *focal_length_mm = focal;
                }
                _ => return Reply::Failed("this camera is orthographic or 360°, which have no focal length".into()),
            }
        }
        self.camera = orbit;
        match self.record_camera() {
            Ok(()) => Reply::Value(self.answer(Query::Camera)),
            Err(error) => Reply::Failed(error),
        }
    }

    /// The command line's options for a render of the session: as the
    /// application renders the project, at the size asked.
    fn render_options(&self, width: Option<usize>, height: Option<usize>, samples: Option<u32>) -> Options {
        let mut options = self.options.clone();
        // Rendered as a project is: its camera, lighting, look and settings.
        options.input = Some(self.path.as_deref().map(|p| p.display().to_string()).unwrap_or_else(|| "session.tequila".into()));
        options.width = width.or(options.width);
        options.height = height.or(options.height);
        options.samples = samples.or(options.samples);
        options
    }

    fn render(&mut self, path: &str, width: usize, height: usize, samples: u32) -> Reply {
        let options = self.render_options(Some(width), Some(height), Some(samples));
        let start = std::time::Instant::now();
        match render_one(&options, &self.document, path, None) {
            Ok(()) => {
                let mut out = Object::new();
                out.set("path", absolute(Path::new(path)))
                    .set("width", width as f64)
                    .set("height", height as f64)
                    .set("samples", samples as f64)
                    .set("seconds", start.elapsed().as_secs_f64());
                Reply::Value(out.build())
            }
            Err(error) => Reply::Failed(error),
        }
    }

    fn export(&mut self, format: &str, path: &str, live: bool, photos: bool, spin: (Option<usize>, Option<usize>, Option<f32>), transparent: bool) -> Reply {
        let mut written = PathBuf::from(path);
        let result: Result<(usize, Vec<String>), String> = match format {
            "glb" | "gltf" => {
                let exported = tequila_format::gltf::export(&self.document, &tequila_format::gltf::GltfOptions { every_model_set: false, ..Default::default() });
                std::fs::write(&written, &exported.glb).map(|_| (exported.glb.len(), exported.warnings)).map_err(|e| format!("Could not write {}: {e}", written.display()))
            }
            "usd" | "usda" => {
                let text = tequila_format::usd::export(&self.document);
                std::fs::write(&written, &text).map(|_| (text.len(), Vec::new())).map_err(|e| format!("Could not write {}: {e}", written.display()))
            }
            "web" => {
                let folder = written.clone();
                let images = photos.then(|| tequila_format::web::fresh_images_folder(&folder));
                if images.is_some() && self.document.configurator.is_empty() {
                    return Reply::Failed("photos need a configurator in the project".into());
                }
                let environment = tequila_scene::environment::EnvironmentCache::new().environment(&self.document.environment);
                let web = tequila_format::web::WebOptions { live, prerendered: images.clone().map(|i| ("jpg".to_string(), i)), ..Default::default() };
                match tequila_format::web::write(&folder, &self.document, &environment, &web) {
                    Ok(report) => {
                        written = report.index.clone();
                        match images {
                            Some(images) => {
                                let template = folder.join(images).join("{configuration}.jpg");
                                each_configuration(&self.render_options(None, None, None), &self.document, &template.to_string_lossy()).map(|_| (report.bytes, report.warnings))
                            }
                            None => Ok((report.bytes, report.warnings)),
                        }
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
            "spin" => {
                let defaults = tequila_format::spin::SpinOptions::default();
                let options = tequila_format::spin::SpinOptions {
                    columns: spin.0.unwrap_or(defaults.columns).clamp(1, 360),
                    rows: spin.1.unwrap_or(defaults.rows).clamp(1, 19),
                    tilt: spin.2.unwrap_or(defaults.tilt),
                    extension: if transparent { "png".into() } else { "jpg".into() },
                    ..defaults
                };
                let mut render = self.render_options(None, None, None);
                render.alpha = transparent;
                let (width, height) = (self.document.render.width, self.document.render.height);
                let saved = self.document.cameras.active();
                let title = if self.document.meta.name.is_empty() { "Tequila".to_string() } else { self.document.meta.name.clone() };
                let folder = written.clone();
                let result = tequila_format::spin::write_package(&folder, &title, &options, width, height).map_err(|e| e.to_string()).and_then(|_| {
                    for frame in tequila_format::spin::frames(&saved.camera, saved.target, &options) {
                        let file = folder.join("frames").join(format!("{}.{}", frame.name, options.extension));
                        render_one(&render, &self.document, &file.to_string_lossy(), Some(frame.camera))?;
                    }
                    tequila_format::spin::finish(&folder, &title, &options, width, height).map_err(|e| e.to_string())
                });
                written = folder.join("index.html");
                result.map(|_| (0, Vec::new()))
            }
            other => Err(format!("`{other}` is not an export: glb, usd, web or spin")),
        };
        match result {
            Ok((bytes, warnings)) => {
                let mut out = Object::new();
                out.set("path", absolute(&written))
                    .set("bytes", bytes as f64)
                    .set("warnings", Value::Array(warnings.into_iter().map(Value::String).collect()))
                    .set("jobs", Value::Array(Vec::new()));
                Reply::Value(out.build())
            }
            Err(message) => Reply::Failed(message),
        }
    }

    /// The application's batch: one undo step, taken back whole on a
    /// failure when atomic.
    fn batch(&mut self, operations: &[Operation], atomic: bool) -> Reply {
        if operations.iter().any(|op| matches!(op, Operation::Quit { .. })) {
            return Reply::Failed("quit cannot be part of a batch; send it on its own".into());
        }
        let total = operations.len();
        self.bus.begin_group();
        let mut results = Vec::with_capacity(total);
        for (index, operation) in operations.iter().enumerate() {
            match self.perform(operation) {
                Reply::Done => results.push(Value::Null),
                Reply::Value(value) => results.push(value),
                Reply::Failed(message) => {
                    let outcome = if atomic {
                        let undone = self.bus.abandon_group(&mut self.document).unwrap_or(0);
                        self.follow_camera();
                        format!("The batch was taken back ({undone} change{} undone), so nothing it did remains.", if undone == 1 { "" } else { "s" })
                    } else {
                        self.bus.end_group();
                        format!("The {index} before it were kept, as one undo step.")
                    };
                    return Reply::Failed(format!("operation {} of {total} ({}) failed: {message}. {outcome}", index + 1, operation.summary()));
                }
            }
        }
        self.bus.end_group();
        let mut out = Object::new();
        out.set("results", Value::Array(results)).set("count", total as f64);
        Reply::Value(out.build())
    }

    /// A new, empty scene, or the project at a path - the open one's unsaved
    /// work settled first, as `quit` settles it.
    fn project(&mut self, action: &tequila_scripting::protocol::ProjectAction, saving: bool, discard: bool) -> Reply {
        use tequila_scripting::protocol::ProjectAction;
        let loaded = match action {
            ProjectAction::New => None,
            ProjectAction::Open(path) => match tequila_format::load(Path::new(path)) {
                Ok(loaded) => Some((loaded, PathBuf::from(path))),
                Err(error) => return Reply::Failed(format!("{path}: {error}")),
            },
        };
        if self.bus.has_unsaved_changes() {
            if saving {
                if let Reply::Failed(message) = self.save(None) {
                    return Reply::Failed(format!("nothing was changed, because the save failed: {message}"));
                }
            } else if !discard {
                return Reply::Failed("there are unsaved changes; send save:true to save them first, or discard:true to throw them away".into());
            }
        }
        let mut out = Object::new();
        match loaded {
            None => {
                self.document = SceneDocument::new();
                self.preserved = tequila_format::Preserved::default();
                self.path = None;
                out.set("project", "new");
            }
            Some((loaded, path)) => {
                self.document = loaded.document;
                self.preserved = loaded.preserved;
                out.set("path", absolute(&path)).set("nodes", self.document.tree.len() as f64);
                self.path = Some(path);
            }
        }
        self.bus = CommandBus::new();
        self.selection.clear();
        self.camera.adopt(self.document.cameras.active());
        self.generation += 1;
        Reply::Value(out.build())
    }

    fn quit(&mut self, saving: bool, discard: bool) -> Reply {
        let mut saved = None;
        if self.bus.has_unsaved_changes() {
            if saving {
                match self.save(None) {
                    Reply::Failed(message) => return Reply::Failed(format!("not closing, because the save failed: {message}")),
                    _ => saved = self.path.as_deref().map(absolute),
                }
            } else if !discard {
                return Reply::Failed("there are unsaved changes; quit with save:true to save them first, or discard:true to throw them away".into());
            }
        }
        self.quit = true;
        let mut out = Object::new();
        out.set("closing", true);
        if let Some(path) = saved {
            out.set("saved", path);
        }
        Reply::Value(out.build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scripting::protocol::parse_request;

    fn ask(session: &mut Session, line: &str) -> Reply {
        let request = parse_request(line).unwrap_or_else(|e| panic!("{line}: {e}"));
        session.handle(&request)
    }

    fn value(session: &mut Session, line: &str) -> Value {
        match ask(session, line) {
            Reply::Value(v) => v,
            Reply::Done => Value::Null,
            Reply::Failed(e) => panic!("{line}: {e}"),
        }
    }

    fn session() -> Session {
        Session::open(&Options::default()).expect("an empty session")
    }

    /// A headless session starts over or opens another project when told
    /// what to do with unsaved work, and not otherwise.
    #[test]
    fn a_headless_session_opens_and_starts_projects() {
        let mut s = session();
        let folder = std::env::temp_dir().join(format!("tequila-serve-open-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("other.tequila");
        tequila_format::save(&file, &tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3), &tequila_format::Preserved::default()).unwrap();
        let file = file.display().to_string().replace('\\', "/");
        value(&mut s, r#"{"op":"apply","command":{"command":"scene.add_group","name":"Unsaved"}}"#);
        assert!(matches!(ask(&mut s, &format!(r#"{{"op":"open","path":"{file}"}}"#)), Reply::Failed(_)), "unsaved work thrown away");
        let opened = value(&mut s, &format!(r#"{{"op":"open","path":"{file}","discard":true}}"#));
        assert!(opened.get("nodes").and_then(Value::as_f64).unwrap_or(0.0) >= 2.0);
        assert!(s.document.tree.iter().any(|n| n.name == "Demo"));
        assert!(!s.bus.has_unsaved_changes());
        value(&mut s, r#"{"op":"new"}"#);
        assert_eq!(s.document.tree.len(), 0, "a new project is empty here");
        assert!(matches!(ask(&mut s, r#"{"op":"open","path":"C:/nowhere/at/all.tequila","discard":true}"#), Reply::Failed(_)));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_headless_session_edits_undoes_and_saves() {
        let mut s = session();
        let model = concat!(env!("CARGO_MANIFEST_DIR"), "/../tequila-import/tests/data/two_bodies.step").replace('\\', "/");
        let imported = value(&mut s, &format!(r#"{{"op":"import","path":"{model}"}}"#));
        assert!(imported.get("nodes").and_then(Value::as_f64).unwrap_or(0.0) > 0.0);
        // The CAD source kept, as the application keeps it.
        assert!(s.document.sources.iter().next().is_some());

        let nodes = value(&mut s, r#"{"op":"query","what":"nodes"}"#);
        let part = nodes.get("nodes").unwrap().as_array().iter().find(|n| n.get("kind").and_then(Value::as_str) == Some("part")).unwrap().clone();
        let id = part.get("id").and_then(Value::as_f64).unwrap() as u64;
        let renamed = format!(r#"{{"op":"apply","command":{{"command":"scene.rename","node":{id},"name":"Lid"}}}}"#);
        assert!(matches!(ask(&mut s, &renamed), Reply::Done));
        assert_eq!(s.document.tree.get(NodeId(id)).unwrap().name, "Lid");
        let undone = value(&mut s, r#"{"op":"undo"}"#);
        assert_eq!(undone.get("undone").and_then(Value::as_str), Some("scene.rename"));
        assert_ne!(s.document.tree.get(NodeId(id)).unwrap().name, "Lid");

        // Unsaved work is not thrown away by a bare quit.
        assert!(matches!(ask(&mut s, r#"{"op":"quit"}"#), Reply::Failed(_)));
        let folder = std::env::temp_dir().join(format!("tequila-serve-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("headless.tequila").display().to_string().replace('\\', "/");
        let saved = value(&mut s, &format!(r#"{{"op":"save","path":"{file}"}}"#));
        assert!(saved.get("path").is_some());
        let reopened = tequila_format::load(Path::new(&file)).expect("the saved project opens");
        assert_eq!(reopened.document.tree.iter().count(), s.document.tree.iter().count());
        assert!(!reopened.document.sources.iter().collect::<Vec<_>>().is_empty(), "the CAD source was saved with it");
        std::fs::remove_dir_all(&folder).ok();
        assert!(matches!(ask(&mut s, r#"{"op":"quit"}"#), Reply::Value(_)));
        assert!(s.quit);
    }

    #[test]
    fn a_failed_batch_is_taken_back_whole() {
        let mut s = session();
        value(&mut s, r#"{"op":"apply","command":{"command":"scene.add_group","parent":null,"name":"Kept?"}}"#);
        let before = s.document.tree.iter().count();
        let reply = ask(&mut s, r#"{"op":"batch","ops":[{"op":"apply","command":{"command":"scene.add_group","parent":null,"name":"A"}},{"op":"select","nodes":[999999]}]}"#);
        assert!(matches!(reply, Reply::Failed(_)));
        assert_eq!(s.document.tree.iter().count(), before, "the batch left a group behind");
    }

    #[test]
    fn framing_and_looking_move_the_saved_camera_undoably() {
        let mut s = session();
        let model = concat!(env!("CARGO_MANIFEST_DIR"), "/../tequila-import/tests/data/two_bodies.step").replace('\\', "/");
        value(&mut s, &format!(r#"{{"op":"import","path":"{model}"}}"#));
        let before = s.document.cameras.active().camera;
        value(&mut s, r#"{"op":"frame","all":true}"#);
        let framed = s.document.cameras.active().camera;
        assert_ne!(before, framed, "framing left the camera where it was");
        // Framing keeps the direction, which a new scene's camera has from
        // the front; looking down from the top is a move of its own.
        let looked = value(&mut s, r#"{"op":"look","view":"top"}"#);
        assert!(looked.get("pitch").and_then(Value::as_f64).is_some_and(|pitch| pitch > 80.0), "{looked:?}");
        assert_ne!(s.document.cameras.active().camera, framed, "the look did not reach the document");
        value(&mut s, r#"{"op":"undo"}"#);
        assert_eq!(s.document.cameras.active().camera, framed, "undo did not take the look back");
    }

    #[test]
    fn what_needs_a_window_is_refused_by_name() {
        let mut s = session();
        let Reply::Failed(message) = ask(&mut s, r#"{"op":"screenshot","path":"x.png"}"#) else { panic!("a screenshot without a window") };
        assert!(message.contains("window"), "{message}");
        let api = value(&mut s, r#"{"op":"query","what":"api"}"#);
        let operations: Vec<&str> = api.get("operations").unwrap().as_array().iter().filter_map(Value::as_str).collect();
        assert!(!operations.contains(&"screenshot") && operations.contains(&"render"));
        let ping = value(&mut s, r#"{"op":"ping"}"#);
        assert_eq!(ping.get("headless").and_then(Value::as_bool), Some(true));
    }
}
