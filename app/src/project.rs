//! The open project: the document, the command bus, the selection, and the
//! crash journal.
//!
//! `TEQUILA_SPEC.md` §2.2: **nothing mutates the Scene Document except the
//! Command Bus.** This type is where that rule meets the application. The
//! document is private, every change goes through [`Project::apply`], and that
//! one funnel is also where the journal is written, the selection is pruned
//! and the render snapshot is invalidated — three things that must happen
//! after *every* edit and would otherwise be three things to remember at
//! forty call sites.

use tequila_render_core::environment::Environment;
use tequila_render_core::scene::SceneSnapshot;
use tequila_render_core::scheduler::{Dirty, SceneChange};
use tequila_color::DisplayPipeline;
use tequila_scene::document::OcioSelection;
use tequila_scene::environment::EnvironmentCache;
use tequila_scene::selection::Selection;
use tequila_scene::snapshot::{MeshCache, SnapshotMapping};
use tequila_scene::{Command, CommandBus, CommandError, MaterialKey, SceneDocument};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// How often §4.5's auto-save fires. The spec's default.
const AUTOSAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// An import with more triangles than this goes into a snapshot written
/// aside rather than into the journal: its line would take a fifth of a
/// second to write and more for each further million.
const HEAVY_TRIANGLES: usize = 250_000;

/// A model read and prepared on its own thread, its meshes converted for the
/// renderer there too: see [`Project::import_prepared`].
pub struct PreparedImport {
    prepared: tequila_import::Prepared,
    converted: Vec<Arc<tequila_render_core::scene::Mesh>>,
}

impl PreparedImport {
    /// Everything of an import that needs no document. Slow on a big model,
    /// so for the import's own thread.
    pub fn prepare(model: tequila_import::ReadModel) -> Result<PreparedImport, tequila_import::ImportError> {
        let prepared = model.prepare(&tequila_import::ImportOptions::default())?;
        let converted = prepared.meshes().iter().map(|mesh| Arc::new(tequila_scene::snapshot::render_mesh_of(mesh))).collect();
        Ok(PreparedImport { prepared, converted })
    }
}

/// An auto-save being written on another thread, and what the journal was
/// when its copy was made.
struct PendingAutosave {
    pending: tequila_format::journal::PendingSnapshot,
    writer: std::thread::JoinHandle<Result<(), String>>,
    /// The undo depth at the copy: where the journal starts once it is used.
    depth: usize,
    epoch: u64,
}

pub struct Project {
    document: SceneDocument,
    bus: CommandBus,
    pub selection: Selection,

    /// Builds the renderer's environment from the document's description, and
    /// keeps the expensive part of it across edits.
    environment: EnvironmentCache,
    /// How the view differs from the document: the graph editor's solo.
    view: tequila_scene::snapshot::ViewOverrides,
    scene: Arc<SceneSnapshot>,
    mapping: SnapshotMapping,
    /// Meshes already converted for the renderer. Kept across snapshots so a
    /// scene that only moved reuses them, which is what lets the backend
    /// update instance transforms in place instead of rebuilding every BVH.
    meshes: MeshCache,
    /// Bumped on every rebuild, so the render thread can tell "same scene"
    /// from "same pointer by luck" without comparing 4,000 instances.
    generation: u64,

    /// Where `Save` writes, once the user has chosen.
    pub path: Option<PathBuf>,
    /// Fields a newer build wrote that this one must not destroy (§4.4).
    preserved: tequila_format::Preserved,

    /// The last colour config built, and what it was built from. A config is
    /// YAML and LUT files; rebuilding one per frame is not an option.
    ocio: Option<(OcioSelection, Option<std::sync::Arc<tequila_color::ocio::Processor>>)>,

    /// The render queue, which spans projects and so is not document state
    /// (`docs/04` §11). It lives beside the crash journal for the same reason:
    /// three jobs from one file and two from another is an ordinary evening,
    /// and a queue kept per project would be each project remembering its own
    /// share of a list that is really one list.
    queue: tequila_scene::render::RenderQueue,
    queued_snapshots: std::collections::HashMap<u64, crate::batch::BatchJob>,
    document_epoch: u64,
    /// Where the queue is written. `None` until a session begins.
    queue_directory: Option<PathBuf>,

    session: Option<tequila_format::journal::Session>,
    /// The undo depth the journal starts from: the history as it stood at the
    /// last save, open or auto-save. Recovery replays onto that state with a
    /// fresh history, so it can undo only what the journal itself applied —
    /// an undo reaching below this point is recorded by snapshot instead.
    journal_depth: usize,
    /// How many redos recovery could replay: undos journaled since the
    /// journal started that nothing has since truncated. A redo beyond these
    /// re-applies something recovery never saw undone, and is recorded by
    /// snapshot instead.
    journal_redos: usize,
    last_autosave: std::time::Instant,
    /// An auto-save being written on another thread.
    autosave: Option<PendingAutosave>,
    /// See [`Project::rebuilds`].
    rebuilds: u64,
    /// An auto-save is wanted now, not in five minutes: a big import is in
    /// the document and not in the journal.
    snapshot_soon: bool,
    /// How many triangles make an import too big for the journal.
    heavy_triangles: usize,
    /// Counts every change to where the journal starts, and every undo or
    /// redo it records: an auto-save begun under one count describes a
    /// journal that a change of it has left behind.
    journal_epoch: u64,
    /// The most recent thing worth telling the user, shown in the status bar.
    pub status: String,
    /// The Studio applied last, by its file-name suffix or name: what
    /// `{studio}` and `{view}` in a render's file name stand for.
    pub last_studio: Option<String>,
}

impl Project {
    /// Open with a document in hand. There is no `new` taking none: an empty
    /// project is `with_document(SceneDocument::new(), ..)`, and having one
    /// constructor keeps the session, snapshot and selection set up in exactly
    /// one place.
    pub fn with_document(document: SceneDocument, environment: Arc<Environment>) -> Project {
        let mut meshes = MeshCache::new();
        // The environment handed in seeds the cache only as a starting image;
        // everything the document says about rotation, brightness, size and
        // the ground is applied on top of it, so a project opened from a file
        // looks the way the file says and not the way the caller guessed.
        let _ = environment;
        let mut environment = EnvironmentCache::new();
        let view = tequila_scene::snapshot::ViewOverrides::default();
        let (scene, mapping) = build_scene(&document, &mut environment, &mut meshes, &view);
        Project {
            view,
            document,
            bus: CommandBus::new(),
            selection: Selection::new(),
            environment,
            scene: Arc::new(scene),
            mapping,
            meshes,
            generation: 1,
            path: None,
            preserved: Default::default(),
            ocio: None,
            queue: tequila_scene::render::RenderQueue::new(),
            queued_snapshots: Default::default(),
            document_epoch: 0,
            queue_directory: None,
            session: None,
            journal_depth: 0,
            journal_redos: 0,
            last_autosave: std::time::Instant::now(),
            autosave: None,
            rebuilds: 0,
            snapshot_soon: false,
            heavy_triangles: HEAVY_TRIANGLES,
            journal_epoch: 0,
            status: String::new(),
            last_studio: None,
        }
    }

    /// Start journalling to `directory` (§4.5). Failing to open the journal is
    /// reported and not fatal: a session without crash recovery is worse than
    /// one with it, and much better than refusing to start.
    pub fn begin_session(&mut self, directory: &Path, id: &str) {
        match tequila_format::journal::Session::open(directory, id, 3) {
            Ok(session) => self.session = Some(session),
            Err(error) => {
                self.status = format!("Crash recovery is unavailable: {error}");
            }
        }
        let (queue, complaint) = tequila_format::queue::load(directory);
        self.queue = queue;
        self.queue_directory = Some(directory.to_path_buf());
        if let Some(complaint) = complaint {
            self.status = complaint;
        }
    }

    pub fn queue(&self) -> &tequila_scene::render::RenderQueue {
        &self.queue
    }

    pub fn queue_mut(&mut self) -> &mut tequila_scene::render::RenderQueue {
        &mut self.queue
    }

    /// Add a job, and write the queue out.
    ///
    /// Saved on every change rather than at exit, because the thing a queue
    /// protects against is the application not reaching its exit.
    pub fn enqueue(
        &mut self,
        name: impl Into<String>,
        camera: usize,
        settings: tequila_scene::render::RenderSettings,
    ) -> u64 {
        self.enqueue_configuration(name, camera, settings, String::new())
    }

    /// [`Project::enqueue`] for one combination of the configurator, which
    /// `{configuration}` in the file name stands for.
    pub fn enqueue_configuration(
        &mut self,
        name: impl Into<String>,
        camera: usize,
        settings: tequila_scene::render::RenderSettings,
        configuration: String,
    ) -> u64 {
        let saved = self.document.cameras.get(camera).cloned();
        self.enqueue_from(name, camera, saved, settings, configuration)
    }

    /// [`Project::enqueue`] from a camera of the caller's own rather than a
    /// saved one: a spin viewer's frames, each from a generated camera.
    pub fn enqueue_camera(
        &mut self,
        name: impl Into<String>,
        camera: tequila_scene::lighting::SavedCamera,
        settings: tequila_scene::render::RenderSettings,
    ) -> u64 {
        let active = self.document.cameras.active_index();
        self.enqueue_from(name, active, Some(camera), settings, String::new())
    }

    fn enqueue_from(
        &mut self,
        name: impl Into<String>,
        camera: usize,
        saved: Option<tequila_scene::lighting::SavedCamera>,
        settings: tequila_scene::render::RenderSettings,
        configuration: String,
    ) -> u64 {
        let name = name.into();
        let id = self.queue.push(name.clone(), camera, settings.clone());
        if let Some(saved) = saved {
            let scene = self.scene();
            let layers = settings.layers.iter().filter(|l| l.enabled).map(|layer| {
                (layer.name.clone(), Arc::new(tequila_scene::snapshot::for_layer(
                    &scene, self.mapping(), &self.document.tree, layer)))
            }).collect();
            let (object_names, material_names) = self.render_names();
            let output_directory = crate::render_window::output_directory(&settings, self.path.as_deref()).display().to_string();
            let job = crate::batch::BatchJob {
                id, name, scene, camera: saved.camera,
                pending_environment: self.environment.is_pending().then(|| self.document.environment.clone()),
                settings: tequila_scene::render::RenderSettings { output_directory, ..settings },
                config: self.integrator_config(), style: self.image_style(), display: self.display_pipeline(),
                object_names, material_names, layers,
                fields: tequila_format::output::NameFields {
                    project: crate::render_window::project_name(&self.document.meta.name, self.path.as_deref()),
                    camera: saved.name, configuration, date: crate::today(), time: crate::now(),
                    studio: self.last_studio.clone().unwrap_or_default(),
                    product: self.product_name(),
                    ..Default::default()
                },
            };
            self.queued_snapshots.insert(id, job);
        }
        self.save_queue();
        id
    }

    pub fn document_epoch(&self) -> u64 { self.document_epoch }

    /// What `{product}` in a file name stands for: the product's one top
    /// group's name, or empty (the project's) when it has several or none.
    pub fn product_name(&self) -> String {
        match tequila_scene::studio::product_roots(&self.document)[..] {
            [root] => self.document.tree.get(root).map(|n| n.name.clone()).unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub fn take_queued_snapshot(&mut self, id: u64) -> Option<crate::batch::BatchJob> {
        self.queued_snapshots.remove(&id)
    }

    pub fn save_queue(&mut self) {
        self.queued_snapshots.retain(|id, _| self.queue.get(*id).is_some_and(|job| !job.state.is_finished()));
        let Some(directory) = self.queue_directory.clone() else {
            return;
        };
        if let Err(error) = tequila_format::queue::save(&directory, &self.queue) {
            self.status = format!("The render queue could not be saved: {error}");
        }
    }

    /// What a render is told before it starts.
    pub fn render_settings(&self) -> &tequila_scene::render::RenderSettings {
        &self.document.render
    }

    /// What each instance and each material is called, by index.
    ///
    /// For the Cryptomatte manifest, which hashes *names*: without these a
    /// matte still selects but a compositor can only offer "click a pixel"
    /// instead of a list of part names, which is most of what it is for.
    ///
    /// An instance with no node behind it — the ground plane is one — is named
    /// for what it is rather than left blank, since an empty name is a
    /// perfectly good hash of nothing that a person cannot recognize.
    pub fn render_names(&self) -> (Vec<String>, Vec<String>) {
        let objects = self
            .scene
            .instances
            .iter()
            .map(|instance| {
                self.mapping
                    .node_of(instance.id)
                    .and_then(|node| self.document.tree.get(node))
                    .map(|node| node.name.clone())
                    .unwrap_or_else(|| {
                        if instance.shadow_catcher.is_some() {
                            "Ground".to_string()
                        } else {
                            format!("Object {}", instance.id.0)
                        }
                    })
            })
            .collect();

        // Slot zero is the neutral grey a part with no material assigned gets,
        // which the document has no name for.
        let mut materials = vec!["Unassigned".to_string()];
        materials.extend(
            self.document
                .materials
                .iter()
                .map(|(_, definition)| definition.name.clone()),
        );
        (objects, materials)
    }

    pub fn document(&self) -> &SceneDocument {
        &self.document
    }

    pub fn scene(&self) -> Arc<SceneSnapshot> {
        self.scene.clone()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Counts the snapshot's rebuilds: every edit to what the scene is -
    /// its tree, transforms, materials - and not the camera moves that also
    /// count as generations. What is worked out from the selection and the
    /// tree holds until this moves.
    pub fn rebuilds(&self) -> u64 {
        self.rebuilds
    }

    pub fn mapping(&self) -> &SnapshotMapping {
        &self.mapping
    }

    pub fn bus(&self) -> &CommandBus {
        &self.bus
    }

    /// The only way to change the document.
    pub fn apply(&mut self, command: Command) -> Result<(), CommandError> {
        // Only a command that writes mesh data can invalidate a converted
        // mesh. A move, a material change or a rename cannot, and keeping the
        // conversions is what lets the renderer update transforms in place
        // rather than rebuilding. Nor can the rest of §9.3's "geometry edits"
        // - lighting, the environment's ground, cutaways, grouping, deleting:
        // they restart the render but leave every mesh as it was, and
        // treating them as mesh edits reconverted and re-uploaded every mesh
        // in the scene on each frame of a slider drag, which froze a
        // 700,000-triangle assembly. Mesh keys are never reused within a
        // document (the store's counter only goes up), so a conversion kept
        // under a key is that key's mesh until one of these rewrites it.
        //
        // An import adds meshes under new keys, and keys only go up between
        // history moves (which clear the cache), so it rewrites none of the
        // meshes already converted: it refits, as the edits below do. It used
        // to clear them all, reconverting every model in the scene each time
        // one more was brought in.
        let rewrites_meshes = matches!(
            command,
            Command::Reimport { .. } | Command::SplitIntoParts { .. } | Command::SetUnits { .. }
        );
        let heavy = self.is_heavy(&command);
        let geometry_edit = command.scene_change() == SceneChange::GeometryEdit;
        let camera_edit = command.scene_change() == SceneChange::CameraTransform;
        // And the other end of the same table: an edit the renderer does not
        // need to see at all must not rebuild the snapshot. That is not an
        // optimization — a fresh snapshot is a fresh `Arc`, the render thread
        // compares by pointer, and a new pointer *is* a restart. Without this,
        // dragging the exposure slider would throw away the accumulated image
        // sixty times a second, which is precisely what §9.3's first row
        // forbids and what M3's exit criterion is stated against.
        let invalidates_render = command.dirty() > Dirty::Display;
        // The journal's line, written once the command applies. Made first,
        // rather than from a copy kept for it: every command used to be
        // copied whole, an imported model's every mesh with it.
        let line = (!heavy && self.session.is_some()).then(|| tequila_format::journal::encode(&command));
        let studio = match &command {
            Command::ApplyStudio { index } => {
                self.document.studios.get(*index).map(|s| s.filename_suffix.clone().unwrap_or_else(|| s.name.clone()))
            }
            _ => None,
        };
        self.bus.apply(&mut self.document, command)?;
        if studio.is_some() {
            self.last_studio = studio;
        }
        if let (Some(session), Some(line)) = (&self.session, line) {
            if let Err(error) = session.record_line(&line) {
                // Not fatal, but the user should know their safety net is
                // gone rather than discovering it after a crash.
                self.status = format!("The journal could not be written: {error}");
            }
        }
        if heavy && self.session.is_some() {
            // A model this big is not written into the journal - a line of
            // hundreds of megabytes, encoded while the window waited - but
            // into a snapshot written aside at once: the journal starts over
            // after it. Until that is written, a crash loses the import, as
            // a crash part way through importing would.
            self.journal_epoch += 1;
            self.journal_depth = self.bus.depth();
            self.journal_redos = 0;
            self.snapshot_soon = true;
        }
        // A new edit truncates redo, here and in any replay of the journal.
        self.journal_redos = 0;
        if rewrites_meshes {
            self.meshes.clear();
        } else if geometry_edit {
            self.meshes.refit();
        }
        if invalidates_render && camera_edit {
            // Notify the viewport without walking the assembly or recompiling
            // its snapshot: camera state travels separately in RenderRequest.
            self.generation += 1;
        } else if invalidates_render {
            self.after_change();
        } else {
            // The selection is still pruned: a command that changes nothing
            // the renderer sees can still remove a node, and a stale selection
            // makes the *next* command refer to something that is gone.
            self.selection.prune(&self.document);
        }
        Ok(())
    }

    pub fn undo(&mut self) {
        if self.bus.undo(&mut self.document).ok().flatten().is_some() {
            // Conservative: undo can restore deleted geometry, and it is not a
            // per-frame operation, so a reconversion costs nothing that
            // matters.
            self.meshes.clear();
            self.after_change();
            if self.bus.depth() < self.journal_depth {
                // Below where the journal starts: recovery has no record of
                // the step being undone, so write the result down whole.
                self.checkpoint();
            } else {
                self.journal_marker(tequila_format::journal::Marker::Undo);
                self.journal_redos += 1;
            }
        }
    }

    pub fn redo(&mut self) {
        if self.bus.redo(&mut self.document).ok().flatten().is_some() {
            self.meshes.clear();
            self.after_change();
            if self.journal_redos > 0 {
                self.journal_marker(tequila_format::journal::Marker::Redo);
                self.journal_redos -= 1;
            } else {
                // Redoing something undone before the journal started, which
                // recovery could not redo: record the result whole.
                self.checkpoint();
            }
        }
    }

    /// An import or update too big to write into the journal line by line:
    /// see [`Project::apply`].
    fn is_heavy(&self, command: &Command) -> bool {
        let triangles: usize = match command {
            Command::Import { payload } => payload.meshes.iter().map(|(_, m)| m.triangle_count()).sum(),
            Command::Reimport { payload } => payload.added.meshes.iter().map(|(_, m)| m.triangle_count()).sum(),
            _ => 0,
        };
        triangles > self.heavy_triangles
    }

    /// Write what the undo history just did into the crash journal.
    fn journal_marker(&mut self, marker: tequila_format::journal::Marker) {
        use tequila_format::journal::Marker;
        // A recovery from an auto-save replays what follows its cut onto a
        // fresh history, where an undo or a taken-back batch from before the
        // cut has nothing to take back. Such a save is not used.
        if matches!(marker, Marker::Undo | Marker::Redo | Marker::AbandonGroup) {
            self.journal_epoch += 1;
        }
        if let Some(session) = &self.session {
            if let Err(error) = session.record_marker(marker) {
                self.status = format!("The journal could not be written: {error}");
            }
        }
    }

    /// Snapshot the document now and start the journal over from it.
    ///
    /// For the history moves a journal cannot express: undoing past its
    /// starting point, redoing something undone before it, recovering a
    /// crashed session whose commands this session's journal never saw.
    fn checkpoint(&mut self) {
        self.journal_epoch += 1;
        let Some(session) = &mut self.session else {
            return;
        };
        match session.snapshot(&self.document) {
            Ok(_) => {
                self.journal_depth = self.bus.depth();
                self.journal_redos = 0;
            }
            Err(error) => self.status = format!("Crash recovery may be out of date: {error}"),
        }
    }

    /// End a continuous interaction, so the next edit starts its own undo
    /// step. See [`CommandBus::end_interaction`].
    pub fn end_interaction(&mut self) {
        self.bus.end_interaction();
    }

    /// Apply what follows as one undo step. See [`CommandBus::begin_group`].
    ///
    /// The group's edges go into the crash journal too: a batch undone after
    /// recovery must come off as one step, as it did before the crash.
    pub fn begin_group(&mut self) {
        self.bus.begin_group();
        self.journal_marker(tequila_format::journal::Marker::BeginGroup);
    }

    pub fn end_group(&mut self) {
        self.bus.end_group();
        self.journal_marker(tequila_format::journal::Marker::EndGroup);
    }

    /// Take back everything applied since the group began. Returns how many
    /// commands that was.
    pub fn abandon_group(&mut self) -> usize {
        let result = self.bus.abandon_group(&mut self.document);
        self.journal_marker(tequila_format::journal::Marker::AbandonGroup);
        if self.bus.depth() < self.journal_depth {
            // A save inside the batch moved the journal's start into the
            // middle of what was just taken back.
            self.checkpoint();
        }
        match result {
            Ok(0) => 0,
            Ok(count) => {
                self.meshes.clear();
                self.after_change();
                count
            }
            Err(error) => {
                self.status = format!("A failed batch could not be fully taken back: {error}");
                self.meshes.clear();
                self.after_change();
                0
            }
        }
    }

    /// The step Ctrl+Z would take back — its last command's name, and how
    /// many commands it holds — if there is one.
    pub fn undo_step(&self) -> Option<(&'static str, usize)> {
        self.bus.undo_step()
    }

    pub fn redo_step(&self) -> Option<(&'static str, usize)> {
        self.bus.redo_step()
    }

    pub fn in_group(&self) -> bool {
        self.bus.in_group()
    }

    pub fn can_undo(&self) -> bool {
        self.bus.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.bus.can_redo()
    }

    /// Show one graph node's output on the model in place of its material,
    /// or `None` to stop (`docs/04` §7's solo). A view setting: not saved,
    /// not undone, and gone when the graph editor closes.
    pub fn set_solo(&mut self, solo: Option<(MaterialKey, tequila_materials::graph::NodeId, u8)>) {
        if self.view.solo != solo {
            self.view.solo = solo;
            self.after_change();
        }
    }

    pub fn solo(&self) -> Option<(MaterialKey, tequila_materials::graph::NodeId, u8)> {
        self.view.solo
    }

    /// Write the composed environment out as a plain HDRI, `width` texels
    /// around: what lights the scene, or with `background` what the camera
    /// sees behind the product.
    pub fn export_environment(
        &mut self,
        path: &std::path::Path,
        width: usize,
        background: bool,
    ) -> Result<(), String> {
        let image = self
            .environment
            .rasterize(&self.document().environment.clone(), width, background);
        tequila_format::hdri::export(&image, path)
    }

    /// The environment as it lights the scene, pins painted in, flattened to
    /// `width` by half: the picture the Lighting pins map moves them on.
    pub fn environment_picture(&mut self, width: usize) -> tequila_texture::Image {
        let settings = self.document().environment.clone();
        self.environment.rasterize(&settings, width, false)
    }

    /// Image pins whose picture would not load.
    pub fn missing_pin_images(&self) -> Vec<String> {
        self.environment.missing_pin_images()
    }

    /// Build new skies and environment images in the background. The app
    /// turns this on; tests leave it off, so an environment asked for is the
    /// environment they get.
    pub fn set_environment_asynchronous(&mut self, asynchronous: bool) {
        self.environment.set_asynchronous(asynchronous);
    }

    /// Pick up a background environment build that has finished, rebuilding
    /// the snapshot when one has. Called once a frame. `true` while one is
    /// still running, so the caller keeps frames coming until it lands.
    pub fn poll_environment(&mut self) -> bool {
        if self.environment.poll() {
            self.after_change();
        }
        self.environment.is_pending()
    }

    /// Rebuild everything that depends on the document.
    ///
    /// Selection is pruned here rather than by each caller: a deleted node
    /// that stays selected makes the next command refer to something that is
    /// not there.
    fn after_change(&mut self) {
        self.rebuilds += 1;
        self.selection.prune(&self.document);
        let (scene, mapping) =
            build_scene(&self.document, &mut self.environment, &mut self.meshes, &self.view);
        self.scene = Arc::new(scene);
        self.mapping = mapping;
        self.generation += 1;
    }

    /// The scene as Studio `index` would show it - its environment, model
    /// set and variants - with its camera, image style and lighting, for its
    /// thumbnail. Built over this project's own converted meshes, and with
    /// `environment` for its environment, so neither the project's meshes
    /// nor its environment are made again; the document is copied without
    /// its geometry, which is lent to the copy while the scene is built.
    /// `None` for no such Studio.
    pub fn studio_scene(
        &mut self,
        index: usize,
        environment: &mut EnvironmentCache,
    ) -> Option<(Arc<SceneSnapshot>, tequila_render_core::camera::Camera, tequila_color::ImageStyle, tequila_render_core::settings::IntegratorConfig)> {
        let geometry = std::mem::take(&mut self.document.geometry);
        let mut copy = self.document.clone();
        self.document.geometry = geometry;
        CommandBus::new().apply(&mut copy, Command::ApplyStudio { index }).ok()?;
        std::mem::swap(&mut copy.geometry, &mut self.document.geometry);
        let (mut scene, _) = build_scene(&copy, environment, &mut self.meshes, &self.view);
        std::mem::swap(&mut copy.geometry, &mut self.document.geometry);
        scene.background = environment.background(&copy.environment);
        Some((Arc::new(scene), copy.cameras.active().camera, copy.image_style, copy.lighting.config()))
    }

    /// What the integrator should run with, from the Lighting tab.
    pub fn integrator_config(&self) -> tequila_render_core::settings::IntegratorConfig {
        self.document.lighting.config()
    }

    /// The grade applied to the accumulated image. Changing it never costs a
    /// ray (§9.3).
    pub fn image_style(&self) -> tequila_color::ImageStyle {
        self.document.image_style
    }

    /// What turns scene-linear radiance into display values: the built-in
    /// transform, or the config the document points at.
    ///
    /// Built on demand and cached against the selection, because loading a
    /// config parses YAML and reads LUT files — perfectly fast once, and far
    /// too slow to do while a slider is being dragged.
    pub fn display_pipeline(&mut self) -> DisplayPipeline {
        let selection = self.document.ocio.clone();
        let Some(selection) = selection else {
            return DisplayPipeline::Builtin(self.document.image_style.display);
        };
        if self.ocio.as_ref().map(|(s, _)| s) != Some(&selection) {
            self.ocio = Some((selection.clone(), self.build_ocio(&selection)));
        }
        match self.ocio.as_ref().and_then(|(_, built)| built.clone()) {
            Some(processor) => DisplayPipeline::Ocio {
                processor,
                label: format!("{} / {}", selection.display, selection.view),
            },
            // A config that will not load falls back to the built-in transform
            // rather than to an identity pass. An identity pass would show a
            // scene-linear image, which looks like a broken renderer rather
            // than like a missing file — and `status` already says which it is.
            None => DisplayPipeline::Builtin(self.document.image_style.display),
        }
    }

    fn build_ocio(
        &mut self,
        selection: &OcioSelection,
    ) -> Option<std::sync::Arc<tequila_color::ocio::Processor>> {
        let config = match tequila_color::ocio::Config::load(&selection.config) {
            Ok(config) => config,
            Err(error) => {
                self.status = format!("The colour config could not be read: {error}");
                return None;
            }
        };
        let processor = config.view_pipeline(&selection.display, &selection.view);
        if !processor.is_complete() {
            // Named, not counted. "3 unsupported transforms" sends someone
            // looking; "BuiltinTransform `ACES-OUTPUT - ...`" tells them what
            // to do about it.
            self.status = format!(
                "`{} / {}` uses transforms Tequila cannot build: {}",
                selection.display,
                selection.view,
                processor.unsupported.join(", ")
            );
            return None;
        }
        self.status = format!(
            "Developing through {} / {}",
            selection.display, selection.view
        );
        Some(std::sync::Arc::new(processor))
    }

    /// The camera the viewport looks through.
    pub fn camera(&self) -> &tequila_scene::lighting::SavedCamera {
        self.document.cameras.active()
    }

    /// §4.5's periodic snapshot. Called every frame; fires on its own schedule.
    ///
    /// Written on another thread from a copy: writing a big model here held
    /// the window for up to two seconds every five minutes. The journal is
    /// cut where the copy was made, and recovery moves to the snapshot only
    /// once it is written whole - see [`Project::poll_autosave`].
    pub fn maybe_autosave(&mut self) {
        self.poll_autosave();
        if self.autosave.is_some() || (!self.snapshot_soon && self.last_autosave.elapsed() < AUTOSAVE_INTERVAL) {
            return;
        }
        // Not part way through a batch: recovery replays what follows the
        // cut onto a fresh history, where the batch's start is missing.
        if self.bus.in_group() {
            return;
        }
        self.last_autosave = std::time::Instant::now();
        if !self.bus.has_unsaved_changes() && !self.snapshot_soon {
            return;
        }
        let Some(session) = &mut self.session else {
            return;
        };
        self.snapshot_soon = false;
        let pending = match session.begin_snapshot() {
            Ok(pending) => pending,
            Err(error) => {
                self.status = format!("Auto-save failed: {error}");
                return;
            }
        };
        let document = self.document.clone();
        let path = pending.path().to_path_buf();
        let writer = std::thread::Builder::new()
            .name("autosave".into())
            .spawn(move || tequila_format::save(&path, &document, &tequila_format::Preserved::default()).map_err(|e| e.to_string()));
        match writer {
            Ok(writer) => {
                self.autosave = Some(PendingAutosave { pending, writer, depth: self.bus.depth(), epoch: self.journal_epoch });
            }
            Err(error) => {
                session.abandon_snapshot(&pending);
                self.status = format!("Auto-save failed: {error}");
            }
        }
    }

    /// Take in an auto-save that has finished writing: recovery starts from
    /// it now - unless the journal moved on meanwhile (a save, an open, an
    /// undo), when it describes nothing recovery could use and is dropped.
    pub fn poll_autosave(&mut self) {
        if !self.autosave.as_ref().is_some_and(|a| a.writer.is_finished()) {
            return;
        }
        self.finish_autosave();
    }

    /// Wait for an auto-save being written, and take it in.
    fn finish_autosave(&mut self) {
        let Some(autosave) = self.autosave.take() else { return };
        let written = autosave.writer.join().unwrap_or_else(|_| Err("the auto-save stopped".into()));
        let Some(session) = &mut self.session else {
            let _ = std::fs::remove_file(autosave.pending.path());
            return;
        };
        match written {
            Ok(()) if autosave.epoch == self.journal_epoch => match session.adopt_snapshot(&autosave.pending) {
                Ok(()) => {
                    self.status = "Auto-saved".into();
                    self.journal_depth = autosave.depth;
                    self.journal_redos = 0;
                }
                Err(error) => self.status = format!("Auto-save failed: {error}"),
            },
            Ok(()) => session.abandon_snapshot(&autosave.pending),
            Err(error) => {
                session.abandon_snapshot(&autosave.pending);
                self.status = format!("Auto-save failed: {error}");
            }
        }
    }

    /// Declare the document as it stands to be the saved state.
    ///
    /// For the one edit that is not a user's: framing the camera on a scene
    /// that has just been opened. It goes through the bus like everything else
    /// — the camera is document state, and a framing that could not be undone
    /// would be the one navigation a user could not take back — but a document
    /// nobody has touched must not claim to have unsaved changes, or every
    /// launch ends in a save prompt for work that was never done.
    pub fn mark_clean(&mut self) {
        self.bus.mark_saved();
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.bus.has_unsaved_changes()
    }

    /// See [`CommandBus::mark_unsaved`].
    pub fn mark_unsaved(&mut self) {
        self.bus.mark_unsaved();
    }

    pub fn save(&mut self, path: &Path) -> Result<(), tequila_format::FormatError> {
        tequila_format::save(path, &self.document, &self.preserved)?;
        self.bus.mark_saved();
        self.path = Some(path.to_path_buf());
        self.rebase_journal();
        self.status = format!("Saved to {}", path.display());
        Ok(())
    }

    /// Point the journal at the file on disk and start it over.
    ///
    /// The file now holds everything the journal described, so keeping those
    /// commands would replay them a second time on recovery. What recovery
    /// needs from here on is only what happens *after* this point — and which
    /// file to apply it to.
    fn rebase_journal(&mut self) {
        self.journal_epoch += 1;
        let Some(session) = &self.session else { return };
        match session.rebase(self.path.as_deref()) {
            Ok(()) => {
                self.journal_depth = self.bus.depth();
                self.journal_redos = 0;
            }
            Err(error) => self.status = format!("Crash recovery may be out of date: {error}"),
        }
    }

    pub fn open(&mut self, path: &Path) -> Result<(), tequila_format::FormatError> {
        let loaded = tequila_format::load(path)?;
        self.document_epoch += 1;
        self.document = loaded.document;
        self.preserved = loaded.preserved;
        // The previous document's history describes edits to a document that
        // is no longer open; keeping it would let undo walk into a scene that
        // never existed.
        self.bus.reset();
        self.selection.clear();
        // A different document entirely.
        self.meshes.clear();
        self.path = Some(path.to_path_buf());
        // The previous project's commands describe a document that is no
        // longer open; replaying them on recovery would build a scene that
        // never existed.
        self.rebase_journal();
        self.status = if loaded.notes.is_empty() {
            format!("Opened {}", path.display())
        } else {
            loaded.notes.join(" ")
        };
        self.after_change();
        Ok(())
    }

    /// Import a model, read on another thread by
    /// [`tequila_import::read_file`], as one undoable step.
    ///
    /// A file that does not say what unit it is in is read in the unit the
    /// importer guesses from its size (§3.4), and comes back with the
    /// [`ModelUnits`](crate::model_units::ModelUnits) that applied the guess,
    /// for the window that says so and offers the others.
    pub fn import_read(
        &mut self,
        model: tequila_import::ReadModel,
    ) -> Result<(tequila_import::ImportReport, Option<crate::model_units::ModelUnits>), String> {
        self.import_read_under(model, None)
    }

    /// The same, under `parent`: what a script's `import` does, which used
    /// to leave a guessed unit unapplied, so a millimetre STL a script
    /// brought in arrived a thousand times too big.
    pub fn import_read_under(
        &mut self,
        model: tequila_import::ReadModel,
        parent: Option<tequila_scene::tree::NodeId>,
    ) -> Result<(tequila_import::ImportReport, Option<crate::model_units::ModelUnits>), String> {
        let ready = PreparedImport::prepare(model).map_err(|e| e.to_string())?;
        self.import_prepared_under(ready, parent)
    }

    /// An import prepared on its own thread - processed, and its meshes
    /// converted for the renderer - finished and applied: all that is left
    /// for the window's thread is giving it the document's keys.
    pub fn import_prepared(&mut self, ready: PreparedImport) -> Result<(tequila_import::ImportReport, Option<crate::model_units::ModelUnits>), String> {
        self.import_prepared_under(ready, None)
    }

    fn import_prepared_under(
        &mut self,
        ready: PreparedImport,
        parent: Option<tequila_scene::tree::NodeId>,
    ) -> Result<(tequila_import::ImportReport, Option<crate::model_units::ModelUnits>), String> {
        let PreparedImport { prepared, converted } = ready;
        if let Some((id, bytes)) = &prepared.brep {
            self.document.sources.insert(id.clone(), bytes.clone());
        }
        let name = prepared.name().to_string();
        let options = tequila_import::ImportOptions::default();
        let (command, report) = prepared.finish(&self.document, parent, &options).map_err(|e| e.to_string())?;
        if let Command::Import { payload } = &command {
            for ((key, _), converted) in payload.meshes.iter().zip(converted) {
                self.meshes.adopt(*key, converted);
            }
        }
        let roots = match &command {
            Command::Import { payload } => payload.roots.clone(),
            _ => Vec::new(),
        };
        let guess = Some(report.source_unit)
            .filter(|unit| report.unit_was_guessed && *unit != tequila_core::units::LengthUnit::Unspecified);
        let Some(unit) = guess else {
            self.apply(command).map_err(|e| e.to_string())?;
            return Ok((report, None));
        };
        // The guess goes in with the import: one undo takes both away.
        self.begin_group();
        let applied = self.apply(command).map_err(|e| e.to_string());
        let mut units = applied
            .as_ref()
            .ok()
            .and_then(|_| crate::model_units::ModelUnits::new(&self.document, roots, name))
            .map(crate::model_units::ModelUnits::about_origin);
        if let Some(rescale) = units.as_mut().and_then(|units| units.guess(unit, &self.document)) {
            let _ = self.apply(rescale);
        }
        self.end_group();
        applied?;
        Ok((report, units))
    }

    /// A CAD file's own bytes, kept with the project so the model can be
    /// divided again without it (`tequila_scene::document::Sources`). A
    /// cache rather than an edit, so not a command.
    pub(crate) fn keep_source(&mut self, model: &tequila_import::ReadModel) {
        if let Some((id, bytes)) = &model.brep {
            self.document.sources.insert(id.clone(), bytes.clone());
        }
    }

    /// Update the model under `roots` from a newer version of its file, read
    /// on another thread like an import (`tequila_scene::reimport`).
    pub fn update_read(
        &mut self,
        model: tequila_import::ReadModel,
        roots: &[tequila_scene::tree::NodeId],
    ) -> Result<tequila_scene::reimport::ReimportReport, String> {
        if roots.is_empty() || roots.iter().any(|&r| !self.document.tree.contains(r)) {
            return Err("the model to update is no longer in the project".into());
        }
        self.keep_source(&model);
        let parent = self.document.tree.get(roots[0]).and_then(|n| n.parent);
        // Where the model stands is the user's: an update does not lift it.
        let options = tequila_import::ImportOptions { place_on_ground: false, ..Default::default() };
        let (command, _) = tequila_import::build_read(&self.document, parent, model, &options)
            .map_err(|e| e.to_string())?;
        let tequila_scene::Command::Import { payload } = command else {
            return Err("the file did not read as a model".into());
        };
        let (update, report) = tequila_scene::reimport::plan(&self.document, roots, *payload);
        self.apply(update).map_err(|e| e.to_string())?;
        self.end_interaction();
        Ok(report)
    }

    /// Replay a crashed session's journal onto the current document.
    pub fn recover(&mut self, journal: &Path) -> Result<(), String> {
        let recovery = tequila_format::journal::replay(&mut self.document, &mut self.bus, journal)
            .map_err(|e| e.to_string())?;
        self.meshes.clear();
        self.after_change();
        // The recovered commands are in the crashed session's journal, not
        // this one's. Snapshot, so this session's journal starts from the
        // recovered document — otherwise a crash now would lose the recovery,
        // and an undo of a recovered step would be journaled against a
        // history the replay does not have.
        self.checkpoint();
        self.status = if recovery.skipped.is_empty() {
            format!("Recovered {} commands", recovery.replayed)
        } else {
            format!(
                "Recovered {} commands; {} could not be replayed",
                recovery.replayed,
                recovery.skipped.len()
            )
        };
        Ok(())
    }

    /// A clean exit: the journal is discarded, so the next launch does not
    /// offer to recover work that was never lost.
    pub fn close(&mut self) {
        // Its file is the session's to delete: let the writing finish first.
        self.finish_autosave();
        if let Some(session) = self.session.take() {
            session.discard();
        }
    }

    /// End the session leaving its journal behind, as a crash would, so the
    /// next launch offers the work back: for a close nobody was asked about,
    /// with work unsaved - a Mac's Quit from the application menu or the
    /// Dock, or logging out, ends the application without the window's
    /// close ever being requested.
    pub fn keep_for_recovery(&mut self) {
        self.finish_autosave();
        drop(self.session.take());
    }

    /// Empty the document, keeping the session and the environment.
    ///
    /// Distinct from [`Project::close`], which only discards the crash-recovery
    /// journal and leaves the scene alone. Conflating the two is how a model
    /// opened from the command line ended up sharing the scene with the demo's
    /// 40-metre ground plane.
    ///
    /// The crash journal then continues from an auto-save of the empty
    /// document: left as it was, it held the demo's commands with no origin,
    /// and a crash recovered the demo with the model in it.
    pub fn reset(&mut self) {
        self.reset_to(SceneDocument::new());
        self.checkpoint();
    }

    /// Start a new project from `document`: no history, no file, and the
    /// crash journal continuing from it, as [`Project::reset`] does.
    pub fn start_new(&mut self, document: SceneDocument) {
        self.reset_to(document);
        self.checkpoint();
    }

    /// Replace the document with `document`, with no history and no file.
    pub fn reset_to(&mut self, document: SceneDocument) {
        self.document_epoch += 1;
        self.document = document;
        self.bus = CommandBus::new();
        self.journal_depth = 0;
        self.journal_redos = 0;
        self.path = None;
        self.selection = Default::default();
        self.meshes.clear();
        self.after_change();
    }
}

/// Build the renderer's scene, environment and background together.
///
/// One function rather than three call sites, because the three have to agree:
/// a snapshot built with last frame's environment, or with a background the
/// settings no longer name, is a frame that shows something the document does
/// not say.
fn build_scene(
    document: &SceneDocument,
    environment: &mut EnvironmentCache,
    meshes: &mut MeshCache,
    view: &tequila_scene::snapshot::ViewOverrides,
) -> (SceneSnapshot, SnapshotMapping) {
    let built = environment.environment(&document.environment);
    let (mut scene, mapping) = tequila_scene::snapshot::snapshot_with(document, built, meshes, view);
    scene.background = environment.background(&document.environment);
    (scene, mapping)
}

impl std::fmt::Debug for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Project")
            .field("nodes", &self.document.tree.len())
            .field("generation", &self.generation)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_color::ImageStyle;
    use tequila_core::math::{vec3, Transform};
    use tequila_scene::tree::NodeKind;

    fn demo() -> Project {
        Project::with_document(
            tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2),
            Arc::new(tequila_scene::demo::studio_environment()),
        )
    }

    /// Grouping restarts the render but rewrites no mesh: the renderer is
    /// handed the meshes it already has, not fresh conversions to upload.
    /// Every frame of a lighting or ground slider used to reconvert all of
    /// them.
    #[test]
    fn an_edit_that_rewrites_no_mesh_keeps_the_converted_meshes() {
        let mut project = demo();
        let parts: Vec<_> = project.document().tree.iter().filter(|n| n.mesh().is_some()).map(|n| n.id).collect();
        let before = project.scene();
        project.apply(Command::Group { nodes: parts.clone(), name: "Together".into() }).unwrap();
        let after = project.scene();
        assert!(!Arc::ptr_eq(&before, &after), "the scene is rebuilt");
        for (a, b) in before.meshes.iter().zip(after.meshes.iter()).take(parts.len()) {
            assert!(Arc::ptr_eq(a, b), "a mesh was converted again");
        }
        // And deleting lets a deleted part's conversion go.
        project.apply(Command::DeleteNodes { nodes: parts }).unwrap();
        assert!(project.meshes.len() <= project.document().tree.iter().filter(|n| n.mesh().is_some()).count());
    }

    #[test]
    fn queued_render_keeps_its_scene_camera_and_destination_after_reset() {
        let mut project = demo();
        project.path = Some(PathBuf::from("original/product.tequila"));
        let original_camera = project.camera().camera;
        let original_scene = project.scene();
        let id = project.enqueue("Original", 0, Default::default());
        project.reset();
        let queued = project.take_queued_snapshot(id).unwrap();
        assert_eq!(queued.camera, original_camera);
        assert!(Arc::ptr_eq(&queued.scene, &original_scene));
        assert!(queued.settings.output_directory.contains("original"));
        assert!(project.take_queued_snapshot(id).is_none());
    }

    #[test]
    fn queued_render_captures_pending_environment_settings() {
        let mut project = demo();
        project.set_environment_asynchronous(true);
        project.document.environment.sky = Some(tequila_render_core::sky::Sky {
            turbidity: 9.0, ..Default::default()
        });
        project.after_change();
        let id = project.enqueue("Pending sky", 0, Default::default());
        project.reset();
        let job = project.take_queued_snapshot(id).unwrap();
        assert_eq!(job.pending_environment.unwrap().sky.unwrap().turbidity, 9.0);
    }

    #[test]
    fn document_epoch_changes_for_replacement_not_normal_edits() {
        let mut project = demo();
        let epoch = project.document_epoch();
        let camera = project.camera().clone();
        project.apply(Command::SetCamera { index: 0, camera: Box::new(camera) }).unwrap();
        assert_eq!(project.document_epoch(), epoch);
        project.reset();
        assert_ne!(project.document_epoch(), epoch);
    }

    #[test]
    fn orbit_edits_keep_the_scene_snapshot_and_notify_the_viewport() {
        let mut project = demo();
        let scene = project.scene();
        let generation = project.generation();
        let mut camera = project.camera().clone();
        camera.camera.f_number = 2.8;
        project.apply(Command::SetCamera { index: project.document().cameras.active_index(), camera: Box::new(camera) }).unwrap();
        assert!(Arc::ptr_eq(&scene, &project.scene()));
        assert!(project.generation() > generation);
        assert_eq!(project.camera().camera.f_number, 2.8);
    }

    /// Crash the project now: find its session the way the next launch
    /// would, load the state its journal continues from, and replay.
    fn recover_from_disk(directory: &Path, session: &str) -> SceneDocument {
        // Described rather than found: the session is still running, which
        // the scan for crashed ones rightly passes over.
        let crashed = tequila_format::journal::describe(directory, session);
        assert!(crashed.commands > 0 || crashed.snapshot.is_some(), "the session is recoverable");
        let origin = crashed.document.as_ref().expect("the journal continues from a file");
        let mut document = tequila_format::load(origin).unwrap().document;
        let mut bus = CommandBus::new();
        let recovery = tequila_format::journal::replay(&mut document, &mut bus, &crashed.journal).unwrap();
        assert!(recovery.skipped.is_empty(), "{:?}", recovery.skipped);
        document
    }

    #[track_caller]
    fn same(recovered: &SceneDocument, live: &SceneDocument, when: &str) {
        if let Some(difference) = recovered.difference_from(live) {
            panic!("{when}: recovery differs from what was on screen: {difference}");
        }
    }

    /// Undo and redo, inside the journal and reaching past where it starts,
    /// and a batch — each followed by a crash, and each recovered to exactly
    /// the document that was on screen.
    #[test]
    fn recovery_reproduces_the_undo_history_as_it_stood() {
        let directory = std::env::temp_dir().join(format!("tequila-recovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let mut project = demo();
        project.begin_session(&directory, "crashy");
        let saved = directory.join("project.tequila");
        let ids: Vec<_> = project.document().tree.iter().map(|n| n.id).take(2).collect();
        let rename = |node, name: &str| Command::Rename { node, name: name.into() };

        project.apply(rename(ids[0], "Saved edit")).unwrap();
        project.save(&saved).unwrap();
        project.apply(rename(ids[1], "After save")).unwrap();
        project.undo();
        same(&recover_from_disk(&directory, "crashy"), project.document(), "undo within the journal");

        // Past the save: the journal holds nothing of this edit.
        project.undo();
        same(&recover_from_disk(&directory, "crashy"), project.document(), "undo past the save");

        project.redo();
        same(&recover_from_disk(&directory, "crashy"), project.document(), "redo of an edit from before the journal");
        project.redo();
        same(&recover_from_disk(&directory, "crashy"), project.document(), "redo within the journal");

        project.begin_group();
        project.apply(rename(ids[0], "Batch one")).unwrap();
        project.apply(rename(ids[1], "Batch two")).unwrap();
        project.end_group();
        project.undo();
        same(&recover_from_disk(&directory, "crashy"), project.document(), "a batch undone");

        project.begin_group();
        project.apply(rename(ids[0], "Doomed")).unwrap();
        project.abandon_group();
        same(&recover_from_disk(&directory, "crashy"), project.document(), "a batch taken back");

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// What importing a very large mesh costs, step by step, on the window's
    /// thread: `cargo test --release -p tequila-app -- --ignored --nocapture
    /// large_import_costs`. TEQUILA_LARGE_TRIANGLES sets the size (5M).
    #[test]
    #[ignore]
    fn large_import_costs() {
        use std::io::Write;
        let triangles: usize = std::env::var("TEQUILA_LARGE_TRIANGLES").ok().and_then(|v| v.parse().ok()).unwrap_or(5_000_000);
        let directory = std::env::temp_dir().join(format!("tequila-large-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        // A binary STL of a bumpy sheet, `triangles` facets.
        let path = directory.join("large.stl");
        {
            let mut out = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
            out.write_all(&[0u8; 80]).unwrap();
            out.write_all(&(triangles as u32).to_le_bytes()).unwrap();
            let side = ((triangles / 2) as f64).sqrt().ceil() as usize;
            let height = |x: usize, y: usize| ((x as f32 * 0.05).sin() * (y as f32 * 0.07).cos()) * 2.0;
            let mut written = 0;
            'all: for y in 0..side {
                for x in 0..side {
                    for half in 0..2 {
                        if written == triangles {
                            break 'all;
                        }
                        let corners = if half == 0 { [(x, y), (x + 1, y), (x, y + 1)] } else { [(x + 1, y), (x + 1, y + 1), (x, y + 1)] };
                        out.write_all(&[0u8; 12]).unwrap();
                        for (cx, cy) in corners {
                            for v in [cx as f32, cy as f32, height(cx, cy)] {
                                out.write_all(&v.to_le_bytes()).unwrap();
                            }
                        }
                        out.write_all(&[0u8; 2]).unwrap();
                        written += 1;
                    }
                }
            }
        }
        let mut project = demo();
        project.begin_session(&directory.join("journal"), "large");
        let start = std::time::Instant::now();
        let model = tequila_import::read_file(&path).unwrap();
        let read = start.elapsed();
        let start = std::time::Instant::now();
        let ready = PreparedImport::prepare(model).unwrap();
        let prepared = start.elapsed();
        let start = std::time::Instant::now();
        project.import_prepared(ready).unwrap();
        let window = start.elapsed();
        let journal = std::fs::metadata(directory.join("journal").join("large.jsonl")).map(|m| m.len()).unwrap_or(0);
        let start = std::time::Instant::now();
        project.maybe_autosave();
        let begun = start.elapsed();
        project.finish_autosave();
        eprintln!(
            "{triangles} triangles: on the import's thread, read {:.0} ms and prepared {:.0} ms | on the window's thread: the import {:.0} ms, its snapshot begun {:.0} ms (journal {:.1} MB)",
            read.as_secs_f64() * 1e3,
            prepared.as_secs_f64() * 1e3,
            window.as_secs_f64() * 1e3,
            begun.as_secs_f64() * 1e3,
            journal as f64 / 1e6
        );
        project.close();
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A small OBJ model, written to `directory`.
    fn small_model(directory: &Path, name: &str, lift: f32) -> tequila_import::ReadModel {
        let path = directory.join(format!("{name}.obj"));
        std::fs::write(&path, format!("v 0 {lift} 0\nv 0.1 {lift} 0\nv 0.1 {} 0\nv 0 {} 0.05\nf 1 2 3\nf 1 3 4\n", lift + 0.1, lift + 0.1)).unwrap();
        tequila_import::read_file(&path).unwrap()
    }

    /// An import too big for the journal goes into a snapshot written aside
    /// straight away, not into a journal line: once it is taken in, a crash
    /// recovers exactly what was on screen - the import, what followed it,
    /// an undo of it and a redo.
    #[test]
    fn a_big_import_is_kept_by_a_snapshot_not_the_journal() {
        let directory = std::env::temp_dir().join(format!("tequila-big-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut project = demo();
        project.heavy_triangles = 0;
        project.begin_session(&directory.join("journal"), "big");
        project.save(&directory.join("project.tequila")).unwrap();
        let model = small_model(&directory, "plate", 0.0);
        project.import_read(model).unwrap();
        let journal = std::fs::read_to_string(directory.join("journal").join("big.jsonl")).unwrap();
        assert!(!journal.contains("scene.import"), "the import was written into the journal");
        assert!(project.snapshot_soon, "no snapshot was asked for");
        project.maybe_autosave();
        assert!(project.autosave.is_some(), "the snapshot did not start at once");
        let plate = project.document().tree.iter().find(|n| n.name.starts_with("plate")).unwrap().id;
        project.apply(Command::Rename { node: plate, name: "Renamed while written".into() }).unwrap();
        project.finish_autosave();
        let journal_dir = directory.join("journal");
        same(&recover_from_disk(&journal_dir, "big"), project.document(), "after the big import's snapshot");
        project.undo();
        project.undo();
        same(&recover_from_disk(&journal_dir, "big"), project.document(), "the big import undone");
        project.redo();
        same(&recover_from_disk(&journal_dir, "big"), project.document(), "the big import redone");

        // Another, undone while its snapshot is still being written: the
        // journal never held it, so the undo cannot be a line in it.
        // An edit in the journal first, which an undo wrongly written as a
        // line would take back on recovery.
        project.apply(Command::Rename { node: plate, name: "Before the second".into() }).unwrap();
        project.import_read(small_model(&directory, "second", 0.5)).unwrap();
        project.maybe_autosave();
        project.undo();
        same(&recover_from_disk(&journal_dir, "big"), project.document(), "a big import undone before its snapshot is in");
        project.finish_autosave();
        same(&recover_from_disk(&journal_dir, "big"), project.document(), "and after it");
        project.close();
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The meshes an import converted on its own thread are the ones the
    /// scene renders, and a second import leaves the first model's
    /// conversions as they were rather than converting everything again.
    #[test]
    fn an_import_brings_its_own_conversions_and_keeps_the_others() {
        let directory = std::env::temp_dir().join(format!("tequila-import-conversions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut project = demo();
        let ready = PreparedImport::prepare(small_model(&directory, "first", 0.0)).unwrap();
        let made: Vec<_> = ready.converted.clone();
        project.import_prepared(ready).unwrap();
        let scene = project.scene();
        for converted in &made {
            assert!(scene.meshes.iter().any(|m| Arc::ptr_eq(m, converted)), "a conversion from the import's thread was made again");
        }
        let before: Vec<_> = scene.meshes.clone();
        project.import_read(small_model(&directory, "second", 0.5)).unwrap();
        let after = project.scene();
        for mesh in before.iter().filter(|m| made.iter().any(|c| Arc::ptr_eq(c, m))) {
            assert!(after.meshes.iter().any(|m| Arc::ptr_eq(m, mesh)), "the first model was converted again for the second");
        }
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Start an auto-save now, as five minutes passing would.
    fn autosave_now(project: &mut Project) {
        project.last_autosave = std::time::Instant::now() - AUTOSAVE_INTERVAL - std::time::Duration::from_secs(1);
        project.maybe_autosave();
        assert!(project.autosave.is_some(), "no auto-save began");
    }

    /// An auto-save written on another thread: a crash while it is being
    /// written, and after it is taken in, each recovers exactly what was on
    /// screen - the edits made while it was written included, once.
    #[test]
    fn an_autosave_written_aside_recovers_exactly() {
        let directory = std::env::temp_dir().join(format!("tequila-autosave-aside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let mut project = demo();
        project.begin_session(&directory, "aside");
        let saved = directory.join("project.tequila");
        let ids: Vec<_> = project.document().tree.iter().map(|n| n.id).take(2).collect();
        let rename = |node, name: &str| Command::Rename { node, name: name.into() };
        project.save(&saved).unwrap();

        // A group added: an edit that, replayed twice, leaves two.
        project.apply(Command::AddGroup { parent: None, name: "Before the copy".into() }).unwrap();
        autosave_now(&mut project);
        project.apply(Command::AddGroup { parent: None, name: "While it is written".into() }).unwrap();
        same(&recover_from_disk(&directory, "aside"), project.document(), "a crash while the auto-save is written");

        project.finish_autosave();
        let crashed = tequila_format::journal::describe(&directory, "aside");
        let origin = crashed.document.clone().expect("an origin");
        assert!(origin.file_name().unwrap().to_string_lossy().starts_with("aside."), "recovery does not start from the auto-save: {}", origin.display());
        assert_eq!(crashed.commands, 1, "only the edit after the copy is left to replay");
        same(&recover_from_disk(&directory, "aside"), project.document(), "a crash after it is taken in");

        // On from there: more edits, an undo inside what follows the cut,
        // a batch - and a second auto-save, which must not write over the
        // first while recovery starts from it.
        project.apply(rename(ids[0], "After")).unwrap();
        project.undo();
        same(&recover_from_disk(&directory, "aside"), project.document(), "an undo after the auto-save");
        autosave_now(&mut project);
        let second = project.autosave.as_ref().unwrap().pending.path().to_path_buf();
        assert_ne!(second, origin, "the second auto-save writes over the one recovery starts from");
        project.begin_group();
        project.apply(rename(ids[0], "Batch")).unwrap();
        project.apply(rename(ids[1], "Batch")).unwrap();
        project.end_group();
        same(&recover_from_disk(&directory, "aside"), project.document(), "a batch while the second is written");
        project.finish_autosave();
        same(&recover_from_disk(&directory, "aside"), project.document(), "after the second");
        project.close();
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// An auto-save the journal moved on from while it was written - an
    /// undo of what it copied, a save - is not used: recovery stays where
    /// it was, still exact, and the stale file goes.
    #[test]
    fn an_autosave_overtaken_while_written_is_not_used() {
        let directory = std::env::temp_dir().join(format!("tequila-autosave-overtaken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let mut project = demo();
        project.begin_session(&directory, "overtaken");
        let saved = directory.join("project.tequila");
        let ids: Vec<_> = project.document().tree.iter().map(|n| n.id).take(2).collect();
        let rename = |node, name: &str| Command::Rename { node, name: name.into() };
        project.save(&saved).unwrap();

        project.apply(rename(ids[0], "Copied, then undone")).unwrap();
        autosave_now(&mut project);
        let stale = project.autosave.as_ref().unwrap().pending.path().to_path_buf();
        project.undo();
        project.finish_autosave();
        assert_eq!(tequila_format::journal::describe(&directory, "overtaken").document.as_deref(), Some(saved.as_path()));
        assert!(!stale.exists(), "the unused auto-save was left behind");
        same(&recover_from_disk(&directory, "overtaken"), project.document(), "an auto-save overtaken by an undo");

        project.apply(rename(ids[1], "Copied, then saved")).unwrap();
        autosave_now(&mut project);
        project.save(&saved).unwrap();
        project.apply(rename(ids[0], "After the save")).unwrap();
        project.finish_autosave();
        assert_eq!(tequila_format::journal::describe(&directory, "overtaken").document.as_deref(), Some(saved.as_path()));
        same(&recover_from_disk(&directory, "overtaken"), project.document(), "an auto-save overtaken by a save");
        project.close();
        let _ = std::fs::remove_dir_all(&directory);
    }

    fn first_part(project: &Project) -> tequila_scene::tree::NodeId {
        project
            .document()
            .tree
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Part { .. }))
            .expect("the demo scene has parts")
            .id
    }

    /// M3's exit criterion, at the level this type is responsible for:
    /// "dragging the exposure slider updates at 60 fps with zero accumulation
    /// loss".
    ///
    /// The mechanism is not obvious, which is why it is worth a test. A fresh
    /// snapshot is a fresh `Arc`; the render thread compares scenes by pointer;
    /// so rebuilding the snapshot *is* a restart, whatever the contents. A
    /// command the renderer does not need to see must therefore not rebuild
    /// one — and an image style is exactly that command.
    #[test]
    fn an_image_style_edit_does_not_disturb_the_render() {
        let mut project = demo();
        let scene = project.scene();
        let generation = project.generation();

        for i in 0..30 {
            project
                .apply(Command::SetImageStyle {
                    style: Box::new(ImageStyle {
                        exposure: i as f32 * 0.1,
                        ..Default::default()
                    }),
                })
                .unwrap();
        }

        assert!(
            Arc::ptr_eq(&scene, &project.scene()),
            "an exposure drag rebuilt the scene, which restarts the render"
        );
        assert_eq!(
            project.generation(),
            generation,
            "an exposure drag bumped the scene generation"
        );
        // And the edit did land, or the test proves only that nothing happened.
        assert!((project.image_style().exposure - 2.9).abs() < 1e-5);
    }

    /// The other side of it: an edit the renderer *does* need to see has to
    /// rebuild. A test for the first half alone would pass on a `Project` that
    /// never rebuilt anything.
    #[test]
    fn a_scene_edit_still_rebuilds() {
        let mut project = demo();
        let scene = project.scene();
        let node = first_part(&project);
        project
            .apply(Command::set_transform(
                node,
                Transform::translation(vec3(0.0, 1.0, 0.0)),
            ))
            .unwrap();
        assert!(
            !Arc::ptr_eq(&scene, &project.scene()),
            "a move did not reach the renderer"
        );
    }

    /// The end of the chain that starts at `MeshCache` and finishes at
    /// `EmbreeIntersector::update_transforms`: moving a part must hand the
    /// renderer the *same* mesh allocations, because that is what the backend
    /// compares to decide it can update in place.
    ///
    /// Tested here as well as in `tequila-scene` because the interesting
    /// question is not whether the cache works — it is whether this crate
    /// clears it too eagerly. It did, before the cache existed, on every
    /// single command.
    #[test]
    fn moving_a_part_reuses_the_converted_meshes() {
        let mut project = demo();
        let node = first_part(&project);
        let before = project.scene();

        project
            .apply(Command::set_transform(
                node,
                Transform::translation(vec3(0.0, 3.0, 0.0)),
            ))
            .unwrap();
        let after = project.scene();

        assert_eq!(before.meshes.len(), after.meshes.len());
        for (a, b) in before.meshes.iter().zip(&after.meshes) {
            assert!(
                Arc::ptr_eq(a, b),
                "a move reconverted the meshes, so the renderer will rebuild every BVH"
            );
        }
        assert_ne!(
            before.instances[0].transform, after.instances[0].transform,
            "nothing actually moved, so this proves nothing"
        );
        // And Cycles' live view takes it for a move, not a new scene.
        assert!(
            tequila_render_cycles::translate::only_moved(&before, &after),
            "Cycles would translate the whole scene again for a dragged part"
        );
    }

    /// A material change does not touch geometry either, and neither does a
    /// rename. Clearing on those would put a drag-adjacent edit back on the
    /// slow path for no reason.
    #[test]
    fn non_geometry_edits_also_keep_the_converted_meshes() {
        let mut project = demo();
        let node = first_part(&project);
        let before = project.scene();

        for command in [
            Command::Rename {
                node,
                name: "Renamed".into(),
            },
            Command::set_visibility(node, true),
            Command::UnlinkMaterial { node },
        ] {
            project.apply(command).unwrap();
        }

        let after = project.scene();
        for (a, b) in before.meshes.iter().zip(&after.meshes) {
            assert!(Arc::ptr_eq(a, b));
        }
    }

    /// And an edit that *does* change geometry must reconvert, or the renderer
    /// keeps tracing the version from before the edit.
    #[test]
    fn a_geometry_edit_reconverts_the_meshes() {
        let mut project = demo();
        let node = first_part(&project);
        let before = project.scene();

        project
            .apply(Command::DeleteNodes { nodes: vec![node] })
            .unwrap();
        let after = project.scene();

        assert!(
            after.meshes.len() < before.meshes.len()
                || !before
                    .meshes
                    .iter()
                    .zip(&after.meshes)
                    .all(|(a, b)| Arc::ptr_eq(a, b)),
            "a deletion left the old conversions in place"
        );
    }

    /// A box `x` by `y` by `z` in a file with no unit, as an OBJ is.
    fn unitless_box(name: &str, x: f32, y: f32, z: f32) -> tequila_import::ReadModel {
        let mut text = String::new();
        for i in 0..8 {
            let at = |bit: usize, size: f32| if i & bit == 0 { 0.0 } else { size };
            text += &format!("v {} {} {}\n", at(1, x), at(2, y), at(4, z));
        }
        for face in ["1 3 4 2", "5 6 8 7", "1 2 6 5", "3 7 8 4", "1 5 7 3", "2 4 8 6"] {
            text += &format!("f {face}\n");
        }
        let folder = std::env::temp_dir().join(format!("tequila-units-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join(format!("{name}.obj"));
        std::fs::write(&path, text).unwrap();
        tequila_import::read_file(&path).expect("reads")
    }

    fn model_bounds(project: &Project) -> tequila_core::math::Aabb {
        tequila_scene::snapshot::visible_bounds(project.document())
    }

    /// A model whose file has no unit comes in at the unit its size
    /// suggests rather than as metres. A part 121 by 110 by 25 in a file that
    /// says nothing is millimetres; read as metres it was 121 m across, and
    /// every material sized for real things - cork's grain, wood's rings -
    /// was a thousandth of its size on it. One undo takes the import and the
    /// guess away together.
    #[test]
    fn a_unitless_model_comes_in_at_its_guessed_unit() {
        use tequila_core::units::LengthUnit;
        let mut project = Project::with_document(SceneDocument::new(), Arc::new(tequila_scene::demo::studio_environment()));
        let (report, units) = project.import_read(unitless_box("cork_pad", 121.0, 110.0, 25.0)).expect("imports");
        assert!(report.unit_was_guessed);
        assert_eq!(report.source_unit, LengthUnit::Millimetre);
        let diagonal = |project: &Project| (model_bounds(project).max - model_bounds(project).min).length();
        let millimetres = (121.0f32 * 121.0 + 110.0 * 110.0 + 25.0 * 25.0).sqrt() * 0.001;
        assert!((diagonal(&project) - millimetres).abs() < 1e-5, "came in {} across", diagonal(&project));
        let mut units = units.expect("the guess is shown, to be changed");
        assert_eq!(units.reading(), Some(LengthUnit::Millimetre));
        let floor = model_bounds(&project).min.y;

        // Read as centimetres it is ten times the size, standing where it
        // stood; back to millimetres, it is as it was; and asking again for
        // what it already is changes nothing.
        let rescale = units.read_as(LengthUnit::Centimetre, project.document()).expect("a change");
        project.apply(rescale).unwrap();
        project.end_interaction();
        assert!((diagonal(&project) - millimetres * 10.0).abs() < 1e-4, "{}", diagonal(&project));
        assert!((model_bounds(&project).min.y - floor).abs() < 1e-5, "it moved off the floor");
        // Where a file stating millimetres puts it: its numbers' place, scaled
        // about the origin - not a few centimetres across, sixty metres out.
        assert!(model_bounds(&project).center().length() < 1.0, "it landed at {:?}", model_bounds(&project).center());
        let rescale = units.read_as(LengthUnit::Millimetre, project.document()).expect("a change");
        project.apply(rescale).unwrap();
        project.end_interaction();
        assert!((diagonal(&project) - millimetres).abs() < 1e-5, "{}", diagonal(&project));
        assert!(units.read_as(LengthUnit::Millimetre, project.document()).is_none());

        project.undo();
        project.undo();
        assert!((diagonal(&project) - millimetres).abs() < 1e-5, "undo did not return to the guess");
        project.undo();
        assert!(model_bounds(&project).is_empty(), "the import outlived one undo of it");
    }

    /// A model already in a scene - under a group that is moved and scaled -
    /// is rescaled in the world, about the middle of its base, whatever its
    /// parent does to it.
    #[test]
    fn a_model_in_a_scaled_group_rescales_where_it_stands() {
        use tequila_core::units::LengthUnit;
        let mut project = Project::with_document(SceneDocument::new(), Arc::new(tequila_scene::demo::studio_environment()));
        let (_, units) = project.import_read(unitless_box("bracket", 80.0, 40.0, 20.0)).expect("imports");
        let roots = units.expect("guessed").roots().to_vec();
        project.apply(Command::Group { nodes: roots.clone(), name: "Holder".into() }).unwrap();
        let group = project.document().tree.get(roots[0]).and_then(|n| n.parent).expect("grouped");
        let placed = Transform::translation(vec3(1.0, 0.5, -2.0))
            * Transform::from_matrix(tequila_core::math::Mat4::scale(tequila_core::math::Vec3::splat(3.0))).unwrap();
        project.apply(Command::SetTransform { transforms: vec![(group, placed)] }).unwrap();
        project.end_interaction();
        let before = model_bounds(&project);

        let mut units = crate::model_units::ModelUnits::new(project.document(), roots, "Bracket".into()).expect("measurable");
        let rescale = units.read_as(LengthUnit::Centimetre, project.document()).expect("a change");
        project.apply(rescale).unwrap();
        let after = model_bounds(&project);
        let (was, now) = (before.max - before.min, after.max - after.min);
        assert!((now - was * 0.01).length() < 1e-5, "{was:?} became {now:?}");
        assert!((after.min.y - before.min.y).abs() < 1e-5, "it left its floor: {} to {}", before.min.y, after.min.y);
        let (centre_was, centre_now) = (before.center(), after.center());
        // Relative: the group puts it a hundred and twenty metres out.
        let near = |a: f32, b: f32| (a - b).abs() <= 1e-6 * a.abs().max(1.0) * 4.0;
        assert!(near(centre_was.x, centre_now.x) && near(centre_was.z, centre_now.z), "it moved sideways: {before:?} to {after:?}");
    }
}
