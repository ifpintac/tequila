//! Answering the scripting channel, on the frame loop.
//!
//! [`tequila_scripting`] owns the transport and the wire format; this is the
//! half that knows what a document is. It runs on the UI thread, between
//! frames, so a scripted change lands exactly where a clicked one does: one
//! command, through the bus, journalled and undoable.
//!
//! # The read side is the point
//!
//! Commands only write. Everything in the catalog changes the document, and
//! none of it can answer "what is in this scene" — so a script driving the app
//! through commands alone is working blind, choosing parts by guessing at
//! identifiers it has never seen.
//!
//! That is not hypothetical. A split model arrives as `part 1 … part 355` with
//! no names worth anything, and the only way to say *which* part is the one
//! you mean is to have its size and its position to say it with. So the node
//! query carries world bounds, and that is the field the rest of this exists
//! to deliver.

use crate::TequilaApp;
use tequila_core::math::Vec3;
use tequila_format::json::Value;
use tequila_format::json_write::Object;
use tequila_scene::tree::NodeId;
use tequila_scripting::protocol::{Operation, Query, Reply, Request};

use tequila_scripting::document::{materials, nodes, PROTOCOL};

use tequila_scripting::document::{APP_LIMITS as LIMITS, APP_OPERATIONS as OPERATIONS, APP_QUERIES as QUERIES};

/// Serve one request, and write it down.
///
/// Every request is logged, including the ones that fail and the ones that
/// only read. A channel that can change someone's scene from outside the
/// application is one whose activity has to be visible, or the person in
/// front of it is the last to know.
pub fn handle(app: &mut TequilaApp, request: &Request) -> Reply {
    let summary = request.operation.summary();
    let reply = perform(app, &request.operation);
    let origin = if request.peer.is_empty() {
        "console".to_string()
    } else {
        request.peer.clone()
    };
    let (outcome, detail) = match &reply {
        Reply::Failed(message) => (crate::scripting_tab::Outcome::Failed, message.clone()),
        _ => (crate::scripting_tab::Outcome::Ok, String::new()),
    };
    // Something outside the application is acting on the scene: say so in
    // the console, where a person watching will already be looking. The
    // console's own requests are not echoed, having been typed there. A batch
    // is spelled out step by step, or "batch of 30" is all anyone would see
    // of thirty changes to their scene.
    if !request.peer.is_empty() {
        let failure = (outcome == crate::scripting_tab::Outcome::Failed).then_some(detail.as_str());
        let steps: Vec<String> = match &request.operation {
            Operation::Batch { operations, .. } => operations.iter().map(Operation::summary).collect(),
            _ => Vec::new(),
        };
        app.console
            .remote(&crate::scripting_tab::clock(), &origin, &summary, &steps, failure);
    }
    app.activity.record(&origin, summary, outcome, detail);
    reply
}

/// Keep the viewport's camera in step with the document's after something
/// that may have moved the document's — an undo, a camera command, a batch
/// taken back.
fn follow_camera(app: &mut TequilaApp, before: &tequila_scene::lighting::SavedCamera) {
    if app.project.camera() != before {
        app.camera.adopt(app.project.camera());
    }
}

/// Do one operation. Separate from [`handle`] so a batch can do many under
/// one log entry.
fn perform(app: &mut TequilaApp, operation: &Operation) -> Reply {
    match operation {
        Operation::Ping => {
            let mut out = Object::new();
            out.set("application", "tequila")
                .set("protocol", PROTOCOL)
                .set("pid", std::process::id() as f64);
            Reply::Value(out.build())
        }
        Operation::Apply(command) => {
            let previous_camera = app.project.camera().clone();
            app.project.end_interaction();
            let result = app.project.apply((**command).clone());
            app.project.end_interaction();
            if result.is_ok() {
                follow_camera(app, &previous_camera);
            }
            match result { Ok(()) => Reply::Done, Err(error) => Reply::Failed(error.to_string()) }
        },
        Operation::Undo | Operation::Redo => {
            if app.project.in_group() {
                // An undo inside a batch would take back part of the batch
                // itself, and leave the rest of it applying to a scene the
                // script no longer knows.
                return Reply::Failed("undo and redo cannot be part of a batch".into());
            }
            let undoing = matches!(operation, Operation::Undo);
            let step = if undoing { app.project.undo_step() } else { app.project.redo_step() };
            let previous_camera = app.project.camera().clone();
            if undoing { app.project.undo() } else { app.project.redo() }
            follow_camera(app, &previous_camera);
            // What was taken back, so a script can tell "undid my batch" from
            // "there was nothing to undo" without querying again.
            let mut out = Object::new();
            let key = if undoing { "undone" } else { "redone" };
            match step {
                Some((name, commands)) => {
                    out.set(key, name).set("commands", commands as f64);
                }
                None => {
                    out.set(key, Value::Null).set("commands", 0.0);
                }
            }
            Reply::Value(out.build())
        }
        Operation::Select(ids) => {
            let nodes: Vec<NodeId> = ids.iter().map(|id| NodeId(*id)).collect();
            let missing: Vec<u64> = nodes
                .iter()
                .filter(|id| app.project.document().tree.get(**id).is_none())
                .map(|id| id.0)
                .collect();
            if !missing.is_empty() {
                // Refused rather than silently narrowed: a script that asked
                // for a part which is not there has a bug, and selecting the
                // subset that happens to exist hides it until something later
                // goes wrong for no visible reason.
                return Reply::Failed(format!("no such part: {missing:?}"));
            }
            app.project
                .selection
                .select_many(nodes, tequila_scene::selection::SelectionMode::Replace);
            Reply::Done
        }
        Operation::Query(query) => Reply::Value(answer(app, *query)),
        Operation::Import { path, parent } => import(app, path, *parent),
        Operation::TextureSet { path, nodes } => texture_set(app, path, nodes),
        Operation::Library(request) => library(app, request),
        Operation::StudioPreset(request) => studio_preset(app, request),
        Operation::Material(request) => material(app, request),
        Operation::Plugin(request) => plugin(app, request),
        Operation::Project { action, save: saving, discard } => project(app, action, *saving, *discard),
        Operation::Reimport { node, path, tessellation } => reimport(app, *node, path.as_deref(), tessellation.as_deref()),
        Operation::Save { path } => save(app, path.as_deref()),
        Operation::Frame { all } => {
            // The same routine the Frame buttons call, so a scripted view and
            // a clicked one land in the same place — and recorded now, rather
            // than on the next frame, so it belongs to the batch it is in.
            app.frame(*all);
            record_camera(app);
            Reply::Value(camera_state(app))
        }
        Operation::Screenshot { path, viewport, window } => {
            // A window's capture is written as the window draws, mid-frame:
            // finished since, it is not still pending.
            app.settle_window_capture();
            if app.pending_screenshot.is_some() {
                return Reply::Failed("A screenshot is already pending; wait for it before requesting another".into());
            }
            // One of the windows of its own: the Render window, the graph.
            let window = match window.as_deref().map(|name| (name, crate::windows::Kind::from_key(name))) {
                None => None,
                Some((name, None)) => {
                    let known: Vec<&str> = crate::windows::Kind::ALL.iter().map(|k| k.key()).collect();
                    return Reply::Failed(format!("There is no window called {name:?}; the windows are {}", known.join(", ")));
                }
                Some((_, Some(kind))) if !app.window_open(kind) => {
                    return Reply::Failed(format!("The {} window is not open; open it first", kind.key()));
                }
                // Drawn inside the main window, it is in the main window's
                // screenshot.
                Some((_, Some(kind))) => app.native_windows.then_some(kind),
            };
            // Deferred: egui hands the framebuffer back as an event on a later
            // frame, so the file cannot exist yet. The reply says where it
            // will be and the client waits for it, which is honest about the
            // timing rather than pretending to be synchronous.
            let target = std::path::PathBuf::from(path);
            let where_it_lands = absolute(&target);
            if let Some(kind) = window {
                crate::windows::request_capture(kind, target.clone());
            }
            app.pending_screenshot = Some(target);
            app.screenshot_viewport = *viewport && window.is_none();
            app.screenshot_window = window;
            let mut out = Object::new();
            out.set("path", where_it_lands)
                .set("pending", true)
                .set("viewport", *viewport && window.is_none());
            if let Some(kind) = window {
                out.set("window", kind.key());
            }
            Reply::Value(out.build())
        }
        Operation::Render { path, width, height, wireframe: true, .. } => match wireframe_to_file(app, path, *width, *height) {
            Ok(value) => Reply::Value(value),
            Err(error) => Reply::Failed(error),
        },
        Operation::Render {
            path,
            width,
            height,
            samples,
            ..
        } => match render_to_file(app, path, *width, *height, *samples) {
            Ok(value) => Reply::Value(value),
            Err(error) => Reply::Failed(error),
        },
        Operation::Batch { operations, atomic } => batch(app, operations, *atomic),
        Operation::Quit { save: saving, discard } => quit(app, *saving, *discard),
        Operation::Ui(change) => ui(app, change),
        Operation::Queue(job) => queue(app, job),
        Operation::LiveLink(message) => livelink(app, message),
        Operation::Export { format, path, live, photos, columns, rows, tilt, transparent } => {
            let mut path = std::path::PathBuf::from(path);
            let result = match format.as_str() {
                "glb" | "gltf" => app.export_glb(&path).map(|(bytes, warnings)| (bytes, warnings, Vec::new())),
                "usd" | "usda" => {
                    let text = tequila_format::usd::export(app.project.document());
                    std::fs::write(&path, &text).map(|_| (text.len(), Vec::new(), Vec::new())).map_err(|e| format!("Could not write {}: {e}", path.display()))
                }
                "web" => app.export_web(&path, *live, *photos).map(|(report, jobs)| (report.bytes, report.warnings, jobs)),
                "spin" => {
                    let defaults = tequila_format::spin::SpinOptions::default();
                    let options = tequila_format::spin::SpinOptions {
                        columns: columns.unwrap_or(defaults.columns).clamp(1, 360),
                        rows: rows.unwrap_or(defaults.rows).clamp(1, 19),
                        tilt: tilt.unwrap_or(defaults.tilt),
                        extension: if *transparent { "png".into() } else { "jpg".into() },
                        ..defaults
                    };
                    app.export_spin(&path, &options, *transparent).map(|(folder, jobs)| {
                        path = folder;
                        (0, Vec::new(), jobs)
                    })
                }
                other => Err(format!("`{other}` is not an export: glb, web or spin")),
            };
            match result {
                Ok((bytes, warnings, jobs)) => {
                    let mut out = Object::new();
                    out.set("path", path.display().to_string())
                        .set("bytes", bytes as f64)
                        .set("warnings", warnings.into_iter().map(Value::from).collect::<Vec<_>>())
                        .set("jobs", jobs.iter().map(|&id| Value::from(id as f64)).collect::<Vec<_>>());
                    Reply::Value(out.build())
                }
                Err(message) => Reply::Failed(message),
            }
        }
        Operation::Stop => {
            if !app.batch.is_running() {
                return Reply::Failed("nothing is rendering in the background".into());
            }
            app.batch.cancel();
            let mut out = Object::new();
            out.set("stopping", true).set(
                "note",
                "the render writes what it has; query jobs to see when its file is ready",
            );
            Reply::Value(out.build())
        }
        Operation::Look(look) => self::look(app, look),
    }
}

/// Several operations as one undo step. See [`Operation::Batch`].
fn batch(app: &mut TequilaApp, operations: &[Operation], atomic: bool) -> Reply {
    if operations.iter().any(|op| matches!(op, Operation::Quit { .. })) {
        return Reply::Failed("quit cannot be part of a batch; send it on its own".into());
    }
    let before_camera = app.project.camera().clone();
    let total = operations.len();
    app.project.begin_group();
    let mut results = Vec::with_capacity(total);
    for (index, operation) in operations.iter().enumerate() {
        match perform(app, operation) {
            Reply::Done => results.push(Value::Null),
            Reply::Value(value) => results.push(value),
            Reply::Failed(message) => {
                let outcome = if atomic {
                    let undone = app.project.abandon_group();
                    follow_camera(app, &before_camera);
                    format!(
                        "The batch was taken back ({undone} change{} undone), so nothing it did remains.",
                        if undone == 1 { "" } else { "s" }
                    )
                } else {
                    app.project.end_group();
                    format!("The {index} before it were kept, as one undo step.")
                };
                return Reply::Failed(format!(
                    "operation {} of {total} ({}) failed: {message}. {outcome}",
                    index + 1,
                    operation.summary()
                ));
            }
        }
    }
    app.project.end_group();
    let mut out = Object::new();
    out.set("results", Value::Array(results))
        .set("count", total as f64);
    Reply::Value(out.build())
}

/// Close the application. See [`Operation::Quit`].
fn quit(app: &mut TequilaApp, saving: bool, discard: bool) -> Reply {
    if app.batch.is_running() && !discard {
        return Reply::Failed(
            "a render is running; send stop and wait for it, or quit with discard:true to abandon it"
                .into(),
        );
    }
    let mut saved = None;
    if app.project.has_unsaved_changes() {
        if saving {
            match save(app, None) {
                Reply::Failed(message) => {
                    return Reply::Failed(format!("not closing, because the save failed: {message}"))
                }
                _ => saved = app.project.path.as_deref().map(absolute),
            }
        } else if !discard {
            return Reply::Failed(
                "there are unsaved changes; quit with save:true to save them first, or discard:true to throw them away"
                    .into(),
            );
        }
    }
    app.quit_requested = true;
    let mut out = Object::new();
    out.set("closing", true);
    if let Some(path) = saved {
        out.set("saved", path);
    }
    Reply::Value(out.build())
}

/// A new project, or one opened: the open one's unsaved work settled first,
/// as `quit` settles it. See [`Operation::Project`].
fn project(app: &mut TequilaApp, action: &tequila_scripting::protocol::ProjectAction, saving: bool, discard: bool) -> Reply {
    use tequila_scripting::protocol::ProjectAction;
    if let ProjectAction::Open(path) = action {
        if !std::path::Path::new(path).is_file() {
            return Reply::Failed(format!("{path} is not a file"));
        }
    }
    if app.project.has_unsaved_changes() {
        if saving {
            if let Reply::Failed(message) = save(app, None) {
                return Reply::Failed(format!("nothing was changed, because the save failed: {message}"));
            }
        } else if !discard {
            return Reply::Failed("there are unsaved changes; send save:true to save them first, or discard:true to throw them away".into());
        }
    }
    let mut out = Object::new();
    match action {
        ProjectAction::New => {
            app.start_new_project();
            out.set("project", "new");
        }
        ProjectAction::Open(path) => {
            let path = std::path::PathBuf::from(path);
            if let Err(error) = app.open_project(&path) {
                return Reply::Failed(error);
            }
            out.set("path", absolute(&path)).set("nodes", app.project.document().tree.len() as f64);
        }
    }
    Reply::Value(out.build())
}

/// Every Inspector tab, by the name a script uses for it.
fn tab_named(name: &str) -> Option<crate::ProjectTab> {
    crate::ProjectTab::ALL
        .into_iter()
        .find(|tab| tab.label().eq_ignore_ascii_case(name.trim()))
}

fn ui(app: &mut TequilaApp, change: &tequila_scripting::protocol::UiChange) -> Reply {
    // Checked first, so a request with one bad field changes nothing at all
    // rather than everything up to the bad field.
    // `configurator` is the Studios tab, scrolled to it.
    let reveal = change.tab.as_deref().is_some_and(|t| t.trim().eq_ignore_ascii_case("configurator"));
    let tab = match &change.tab {
        Some(_) if reveal => Some(crate::ProjectTab::Studios),
        Some(name) => match tab_named(name) {
            Some(tab) => Some(tab),
            None => {
                let names: Vec<String> = crate::ProjectTab::ALL
                    .iter()
                    .map(|t| t.label().to_ascii_lowercase())
                    .collect();
                return Reply::Failed(format!("`{name}` is not an Inspector tab; try {}", names.join(", ")));
            }
        },
        None => None,
    };
    let view = match change.viewport.as_deref().map(str::to_ascii_lowercase) {
        None => None,
        Some(name) => match crate::view_mode::ViewMode::from_key(&name) {
            Some(mode) => Some(mode),
            None => return Reply::Failed(format!("the viewport is `wireframe`, `solid`, `material` or `rendered`, not `{name}`")),
        },
    };

    let graph = match change.graph {
        Some(Some(key)) => {
            let key = tequila_scene::tree::MaterialKey(key);
            if app.project.document().materials.get(key).is_none() {
                return Reply::Failed(format!("there is no material {}; query materials for their numbers", key.0));
            }
            Some(Some(key))
        }
        other => other.map(|_| None),
    };

    match graph {
        Some(Some(key)) => app.graph_editor.open_for(key),
        Some(None) => app.graph_editor.open = false,
        None => {}
    }
    if let Some(open) = change.render_window {
        app.render_window.open = open;
    }
    if let Some(open) = change.console {
        app.console.open = open;
    }
    if let Some(show) = change.inspector {
        app.show_project = show;
    }
    if let Some(tab) = tab {
        app.project_tab = tab;
        app.show_project = true;
    }
    if reveal {
        app.reveal_configurator();
    }
    if let Some(show) = change.library {
        app.show_library = show;
    }
    if let Some(mode) = view {
        app.view_mode = mode;
        app.shading = mode.shading();
    }
    if let Some(full) = change.full_viewport {
        app.full_viewport = full;
    }
    if let Some(edges) = change.edges {
        app.show_model_edges = edges;
    }
    if let Some(frame) = change.frame {
        app.show_render_frame = frame;
    }
    match change.present {
        Some(true) => app.start_presentation(),
        Some(false) => app.end_presentation(),
        None => {}
    }
    if let Some(about) = change.about {
        app.show_about = about;
    }
    if let Some(help) = change.help {
        app.show_help = help;
    }
    if let Some(open) = change.start_screen {
        app.start_screen.open = open;
    }
    if let Some(open) = change.connect {
        app.start_screen.open &= !open;
        app.assistant.set_providers_open(open);
    }
    match change.updates {
        Some(true) => {
            app.start_screen.open = false;
            app.updates.open_window();
        }
        Some(false) => app.updates.open = false,
        None => {}
    }
    if let Some(open) = change.appearance {
        app.open_look = Some(open);
    }
    if change.mode.is_some() || change.theme.is_some() {
        let mut look = app.pending_look.unwrap_or_else(crate::theme::appearance);
        if let Some(mode) = &change.mode {
            match crate::theme::Mode::ALL.into_iter().find(|m| m.key() == mode.to_ascii_lowercase()) {
                Some(m) => look.mode = m,
                None => return Reply::Failed(format!("the mode is `light`, `dark` or `auto`, not `{mode}`")),
            }
        }
        if let Some(theme) = &change.theme {
            match crate::theme::Flavor::ALL.into_iter().find(|f| f.key() == theme.to_ascii_lowercase()) {
                Some(f) => look.flavor = f,
                None => {
                    let names: Vec<&str> = crate::theme::Flavor::ALL.iter().map(|f| f.key()).collect();
                    return Reply::Failed(format!("`{theme}` is not a theme; try {}", names.join(", ")));
                }
            }
        }
        app.pending_look = Some(look);
    }
    Reply::Value(ui_state(app))
}

fn ui_state(app: &TequilaApp) -> Value {
    let mut out = Object::new();
    out.set("render_window", app.render_window.open)
        .set("console", app.console.open)
        .set("inspector", app.show_project)
        .set("tab", app.project_tab.label().to_ascii_lowercase())
        .set("library", app.show_library)
        .set("viewport", app.view_mode.key())
        // How far the viewport's picture has got: its samples, and whether
        // Cycles is still preparing the graphics card and has shown nothing.
        .set("viewport_samples", app.image.samples as f64)
        .set("viewport_preparing", matches!(app.engine_note, Some(crate::orchestrator::EngineNote::Preparing(_))))
        .set("full_viewport", app.full_viewport)
        .set("present", app.presenting.is_some())
        .set("edges", app.show_model_edges)
        .set("frame", app.show_render_frame)
        .set("help", app.show_help)
        .set("about", app.show_about)
        .set("recent", Value::Array(app.recent.iter().map(|p| Value::String(p.display().to_string())).collect()))
        .set("mode", app.pending_look.unwrap_or_else(crate::theme::appearance).mode.key())
        .set("theme", app.pending_look.unwrap_or_else(crate::theme::appearance).flavor.key())
        .set(
            "graph",
            match app.graph_editor.material().filter(|_| app.graph_editor.open) {
                Some(key) => Value::Number(key.0 as f64),
                None => Value::Null,
            },
        )
        .set(
            "viewport_size",
            Value::Array(vec![
                Value::Number(app.viewport_size.0 as f64),
                Value::Number(app.viewport_size.1 as f64),
            ]),
        );
    out.build()
}

/// A LiveLink message (`docs/livelink-protocol.md`): a modelling
/// application saying hello, pushing what changed, or moving the camera.
fn livelink(app: &mut TequilaApp, message: &Value) -> Reply {
    let kind = message.get("message").and_then(|m| m.as_str()).unwrap_or("");
    match kind {
        "hello" => {
            let protocol = message.get("protocol").and_then(|v| v.as_index()).unwrap_or(1);
            if protocol != 1 {
                return Reply::Failed(format!("this Tequila speaks LiveLink protocol 1, not {protocol}"));
            }
            let client = message.get("client").and_then(|v| v.as_str()).unwrap_or("a client");
            app.project.status = format!("LiveLink: {client} connected");
            let mut out = Object::new();
            out.set("protocol", 1.0)
                .set("session", std::process::id() as f64)
                .set("scene", app.project.document().meta.name.clone())
                .set("capabilities", ["sync", "camera", "materials", "deletes"].iter().map(|s| Value::from(*s)).collect::<Vec<_>>());
            Reply::Value(out.build())
        }
        "sync" => {
            let sync = match tequila_format::livelink::read_sync(message) {
                Ok(sync) => sync,
                Err(error) => return Reply::Failed(format!("the sync could not be read: {error}")),
            };
            let (command, report) = tequila_scene::livelink::plan(app.project.document(), &sync);
            if let Err(error) = app.project.apply(command) {
                return Reply::Failed(format!("the sync could not be applied: {error}"));
            }
            app.project.status = format!(
                "LiveLink: {} updated, {} added, {} hidden",
                report.matched, report.added, report.removed
            );
            let mut out = Object::new();
            out.set_some("transaction", message.get("transaction").and_then(|v| v.as_f64()))
                .set("matched", report.matched as f64)
                .set("added", report.added as f64)
                .set("removed", report.removed as f64);
            Reply::Value(out.build())
        }
        "camera" => {
            let vector = |key: &str| message.get(key).and_then(|v| v.as_f32_array()).filter(|v| v.len() == 3).map(|v| [v[0], v[1], v[2]]);
            let (Some(from), Some(at)) = (vector("position"), vector("target")) else {
                return Reply::Failed("a camera message needs a position and a target".into());
            };
            let look = tequila_scripting::protocol::Look {
                from: Some(from),
                at: Some(at),
                focal_length: message.get("focal_length").and_then(|v| v.as_f32()),
                ..Default::default()
            };
            self::look(app, &look)
        }
        other => Reply::Failed(format!("`{other}` is not a LiveLink message: hello, sync or camera")),
    }
}

/// A background render. See [`Operation::Queue`].
fn queue(app: &mut TequilaApp, job: &tequila_scripting::protocol::QueueRender) -> Reply {
    let mut settings = app.project.render_settings().clone();
    for (value, target, name) in [
        (job.width, &mut settings.width, "width"),
        (job.height, &mut settings.height, "height"),
    ] {
        if let Some(value) = value {
            if !(16..=16384).contains(&value) {
                return Reply::Failed(format!("{name} must be between 16 and 16384 pixels"));
            }
            *target = value;
        }
    }
    if let Some(samples) = job.samples {
        if samples == 0 {
            return Reply::Failed("samples must be at least 1".into());
        }
        settings.max_samples = samples;
    }
    if let Some(folder) = &job.folder {
        settings.output_directory = folder.clone();
    }
    if let Some(filename) = &job.filename {
        settings.filename_template = filename.clone();
    }
    if let Some(denoise) = job.denoise {
        settings.denoise = denoise;
    }
    if let Some(engine) = &job.engine {
        let Some(engine) = tequila_scene::render::RenderEngine::from_key(engine) else {
            return Reply::Failed(format!("`{engine}` is not an engine: tequila, cycles-cpu or cycles-gpu"));
        };
        settings.engine = engine;
    }
    // A region is kept only if it still fits the image it is now part of.
    if let Some(region) = &settings.region {
        if region.x + region.width > settings.width || region.y + region.height > settings.height {
            settings.region = None;
        }
    }
    if job.network == Some(true) {
        if job.camera.is_some() {
            return Reply::Failed("a network render is of the active camera; select the camera first (camera.select)".into());
        }
        let every = job.configurations == Some(true);
        let folder = crate::render_window::output_directory(&settings, app.project.path.as_deref());
        return match app.render_window.send_to_network(&settings, &mut app.project, every) {
            Ok(count) => {
                let mut out = Object::new();
                out.set("count", count as f64)
                    .set("network", true)
                    .set("folder", folder.display().to_string())
                    .set("note", "sent to the render nodes; the files land in the folder as they finish (Render window, Network rendering)");
                Reply::Value(out.build())
            }
            Err(why) => Reply::Failed(why),
        };
    }
    if job.studios == Some(true) {
        if app.project.document().studios.is_empty() {
            return Reply::Failed("there are no Studios; set them with studio.set or apply a studio preset".into());
        }
        let folder = crate::render_window::output_directory(&settings, app.project.path.as_deref());
        let ids = app.queue_studios(Some(settings));
        let mut out = Object::new();
        out.set("jobs", ids.iter().map(|&id| Value::from(id as f64)).collect::<Vec<_>>())
            .set("count", ids.len() as f64)
            .set("folder", folder.display().to_string())
            .set("note", "one job per Studio, each view captured now; query jobs for progress");
        return Reply::Value(out.build());
    }
    if let Some(wanted) = &job.studio {
        let studios = &app.project.document().studios;
        let Some(index) = studios
            .iter()
            .position(|s| &s.name == wanted)
            .or_else(|| wanted.parse::<usize>().ok().filter(|&i| i < studios.len()))
        else {
            let names: Vec<&str> = studios.iter().map(|s| s.name.as_str()).collect();
            return Reply::Failed(format!("there is no Studio `{wanted}`; there are {}", if names.is_empty() { "none".to_string() } else { names.join(", ") }));
        };
        // Applied for the render and taken back, as a batch of views is.
        let before = app.project.last_studio.clone();
        app.project.begin_group();
        if let Err(error) = app.project.apply(tequila_scene::Command::ApplyStudio { index }) {
            app.project.abandon_group();
            return Reply::Failed(error.to_string());
        }
        let name = job.name.clone().unwrap_or_else(|| app.project.document().studios[index].name.clone());
        let camera = app.project.document().cameras.active_index();
        let (width, height, samples) = (settings.width, settings.height, settings.max_samples);
        let folder = crate::render_window::output_directory(&settings, app.project.path.as_deref());
        let id = app.project.enqueue(name.clone(), camera, settings);
        app.project.abandon_group();
        app.project.last_studio = before;
        app.camera.adopt(app.project.camera());
        app.submit();
        let mut out = Object::new();
        out.set("job", id as f64)
            .set("name", name)
            .set("width", width as f64)
            .set("height", height as f64)
            .set("samples", samples as f64)
            .set("folder", folder.display().to_string())
            .set("note", "the Studio is captured now and the scene left as it was; query jobs for progress and the file");
        return Reply::Value(out.build());
    }
    if job.configurations == Some(true) {
        if app.project.document().configurator.is_empty() {
            return Reply::Failed("there is no configurator; set one with configurator.set".into());
        }
        let folder = crate::render_window::output_directory(&settings, app.project.path.as_deref());
        let ids = app.queue_configurations(Some(settings));
        let mut out = Object::new();
        out.set("jobs", ids.iter().map(|&id| Value::from(id as f64)).collect::<Vec<_>>())
            .set("count", ids.len() as f64)
            .set("folder", folder.display().to_string())
            .set("note", "one job per valid combination, each scene captured now; query jobs for progress");
        return Reply::Value(out.build());
    }
    let cameras = &app.project.document().cameras;
    let camera = job.camera.unwrap_or_else(|| cameras.active_index());
    let Some(saved) = cameras.get(camera) else {
        return Reply::Failed(format!(
            "there is no camera {camera}; this scene has {} (numbered from 0)",
            cameras.len()
        ));
    };
    let name = job.name.clone().unwrap_or_else(|| saved.name.clone());
    let folder = crate::render_window::output_directory(&settings, app.project.path.as_deref());
    let (width, height, samples) = (settings.width, settings.height, settings.max_samples);
    let id = app.project.enqueue(name.clone(), camera, settings);
    let mut out = Object::new();
    out.set("job", id as f64)
        .set("name", name)
        .set("width", width as f64)
        .set("height", height as f64)
        .set("samples", samples as f64)
        .set("folder", folder.display().to_string())
        .set(
            "note",
            "the scene and camera are captured now; query jobs for progress and the file",
        );
    Reply::Value(out.build())
}

fn jobs(app: &TequilaApp) -> Value {
    let list = app
        .project
        .queue()
        .jobs()
        .iter()
        .map(|job| {
            let mut entry = Object::new();
            entry
                .set("id", job.id as f64)
                .set("name", job.name.clone())
                .set("state", job.state.label().to_ascii_lowercase())
                .set("progress", crate::render_window::progress(job))
                .set("samples", job.samples as f64)
                .set("max_samples", job.settings.max_samples as f64)
                .set("seconds", job.seconds)
                .set("width", job.settings.width as f64)
                .set("height", job.settings.height as f64);
            entry.set_some("output", job.output.clone());
            if !job.message.is_empty() {
                entry.set("message", job.message.clone());
            }
            entry.build()
        })
        .collect();
    let mut out = Object::new();
    out.set("jobs", Value::Array(list))
        .set("rendering", app.batch.is_running());
    out.build()
}

fn history(app: &TequilaApp) -> Value {
    let step = |step: Option<(&'static str, usize)>| match step {
        Some((name, commands)) => {
            let mut out = Object::new();
            out.set("command", name).set("commands", commands as f64);
            out.build()
        }
        None => Value::Null,
    };
    let mut out = Object::new();
    out.set("undo", step(app.project.undo_step()))
        .set("redo", step(app.project.redo_step()))
        .set("unsaved", app.project.has_unsaved_changes())
        .set(
            "path",
            app.project
                .path
                .as_deref()
                .map(absolute)
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
    out.build()
}

/// Where the camera is, in the terms `look` takes.
fn camera_state(app: &TequilaApp) -> Value {
    tequila_scripting::document::camera(&app.camera, &app.project.camera().name)
}

/// Put the camera somewhere. See [`Operation::Look`].
fn look(app: &mut TequilaApp, look: &tequila_scripting::protocol::Look) -> Reply {
    let to_vec = |v: [f32; 3]| Vec3 { x: v[0], y: v[1], z: v[2] };
    let mut orbit = app.camera;
    if let Some(name) = &look.view {
        match crate::orbit::OrbitCamera::view_named(name) {
            Some(view) => orbit.set_standard_view(view),
            None => {
                return Reply::Failed(format!(
                    "`{name}` is not a view; try front, back, left, right, top, bottom or iso"
                ))
            }
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
        if let Some(distance) = look.distance {
            if distance <= 0.0 {
                return Reply::Failed("distance must be more than zero".into());
            }
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
            tequila_render_core::camera::Projection::Perspective { focal_length_mm }
            | tequila_render_core::camera::Projection::Shift { focal_length_mm, .. } => {
                *focal_length_mm = focal;
            }
            _ => {
                return Reply::Failed(
                    "this camera is orthographic or 360°, which have no focal length".into(),
                )
            }
        }
    }
    app.camera = orbit;
    record_camera(app);
    Reply::Value(camera_state(app))
}

/// Put the viewport's camera into the document now, as its own undo step.
///
/// Now, rather than on the next frame, so it is undoable and belongs to the
/// batch it arrived in. Its own step, because camera commands merge the way a
/// drag's frames should — and without the barriers, two scripted moves became
/// one undo, and a scripted move merged into the person's own last orbit.
fn record_camera(app: &mut TequilaApp) {
    app.project.end_interaction();
    app.sync_viewport_camera();
    app.project.end_interaction();
}

fn answer(app: &mut TequilaApp, query: Query) -> Value {
    match query {
        Query::Library => tequila_scripting::document::library(&crate::library_tab::user_library_path()),
        Query::Api => tequila_scripting::document::api(OPERATIONS, QUERIES, LIMITS),
        Query::Context => {
            let mut out = Object::new();
            out.set("scene", scene(app));
            out.set("nodes", nodes(app.project.document()));
            out.set("materials", materials(app.project.document()));
            out.set("selection", answer(app, Query::Selection));
            out.set("camera", answer(app, Query::Camera));
            let description = tequila_format::scene_description(app.project.document());
            if let Ok(value) = tequila_format::json::parse(description.as_bytes()) { out.set("document", value); }
            out.set("generation", app.project.generation() as f64);
            out.build()
        }
        Query::Scene => scene(app),
        Query::Nodes => nodes(app.project.document()),
        Query::Materials => materials(app.project.document()),
        Query::Selection => {
            let ids: Vec<Value> = app
                .project
                .selection
                .nodes()
                .map(|id| Value::Number(id.0 as f64))
                .collect();
            let mut out = Object::new();
            out.set("nodes", Value::Array(ids));
            out.build()
        }
        Query::Commands => tequila_scripting::document::commands(),
        Query::Camera => camera_state(app),
        Query::Ui => ui_state(app),
        Query::Jobs => jobs(app),
        Query::History => history(app),
    }
}

fn scene(app: &mut TequilaApp) -> Value {
    tequila_scripting::document::scene(app.project.document(), app.project.path.as_deref())
}

/// Update an imported model from its file, or from `path`: the model `node`
/// belongs to, or the selection's. Read here rather than on a worker, like a
/// scripted import, so the reply can say what it did.
fn reimport(app: &mut TequilaApp, node: Option<u64>, path: Option<&str>, tessellation: Option<&str>) -> Reply {
    let Some(node) = node.map(NodeId).or_else(|| app.project.selection.active()) else {
        return Reply::Failed("name a node of the model, or select one".into());
    };
    if !app.project.document().tree.contains(node) {
        return Reply::Failed(format!("there is no node {}", node.0));
    }
    let roots = tequila_scene::reimport::import_roots(app.project.document(), node);
    let quality = match tessellation.map(|t| (t, tequila_import::step::Tessellation::from_label(t))) {
        None => None,
        Some((_, Some(quality))) => Some(quality),
        Some((name, None)) => return Reply::Failed(format!("`{name}` is not a tessellation: draft, medium, high or ultra")),
    };
    // Divided again from the CAD source kept with the project, when there
    // is one and no other file was named.
    let kept = roots
        .first()
        .and_then(|r| app.project.document().tree.get(*r))
        .and_then(|n| n.metadata.get(tequila_scene::document::BREP))
        .and_then(|id| app.project.document().sources.get(id))
        .cloned();
    let recorded = tequila_scene::reimport::source_file(app.project.document(), &roots).map(std::path::PathBuf::from);
    let read = match (quality, path, kept) {
        (Some(quality), None, Some(bytes)) => {
            let name = recorded.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Model".into());
            Some(tequila_import::read_cad(&bytes, &name, recorded.clone(), quality))
        }
        _ => None,
    };
    let file = match path {
        Some(path) => std::path::PathBuf::from(path),
        None => match tequila_scene::reimport::source_file(app.project.document(), &roots) {
            Some(file) => std::path::PathBuf::from(file),
            None => return Reply::Failed("this model records no file; give a path".into()),
        },
    };
    let read = read.unwrap_or_else(|| tequila_import::read_file_with(&file, quality.unwrap_or_default()));
    let model = match read {
        Ok(model) => model,
        Err(error) => return Reply::Failed(error.to_string()),
    };
    match app.project.update_read(model, &roots) {
        Ok(report) => {
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
        Err(error) => Reply::Failed(error),
    }
}

/// A PBR texture pack as materials, the first put on `nodes` or on the
/// selection. The same route the File menu and the Material tab take.
/// The material library, as a script reaches it: presets put on parts
/// through the bus, like a drop from the browser, and the user's own
/// library added to, exported from or tidied - the browser re-reading it,
/// so the person sees what the script did.
fn library(app: &mut TequilaApp, request: &tequila_scripting::protocol::LibraryRequest) -> Reply {
    use tequila_scripting::document::{library_request, LibraryOutcome};
    let selection: Vec<NodeId> = app.project.selection.nodes().collect();
    let pack = |path: &std::path::Path, size: f32, name: String, category: String, tags: Vec<String>| {
        tequila_import::texture_set::library_entry(path, size, name, category, tags).map_err(|e| e.to_string())
    };
    match library_request(request, app.project.document(), &selection, &crate::library_tab::user_library_path(), &pack) {
        Ok(outcome @ LibraryOutcome::Commands(..)) => {
            let reply = carry_out(app, outcome);
            app.library.note_used(&request.name.clone().unwrap_or_default());
            reply
        }
        Ok(outcome) => {
            app.library.reload();
            carry_out(app, outcome)
        }
        Err(error) => Reply::Failed(error),
    }
}

/// Studio presets (`docs/studio-presets.md`): listed, applied by id, the
/// scene's studio saved as one, installed, exported, deleted, and every view
/// rendered.
fn studio_preset(app: &mut TequilaApp, request: &Value) -> Reply {
    use tequila_format::studio_preset as presets;
    use tequila_scripting::studio_presets as shared;
    let action = request.get("action").and_then(|a| a.as_str()).unwrap_or("").to_ascii_lowercase();
    let path = |what: &str| request.get("path").and_then(|v| v.as_str()).map(std::path::PathBuf::from).ok_or_else(|| format!("`{action}` needs {what} \"path\""));
    let done = |result: Result<(), String>| match result {
        Ok(()) => Reply::Done,
        Err(e) => Reply::Failed(e),
    };
    match action.as_str() {
        "list" => Reply::Value(shared::list(crate::studio_presets::product_radius(&app.project))),
        "apply" => {
            let (file, options) = match shared::named(request, "apply").and_then(|file| shared::options(request).map(|o| (file, o))) {
                Ok(found) => found,
                Err(e) => return Reply::Failed(e),
            };
            let dress = request.get("dress").and_then(|v| v.as_bool()).unwrap_or(false);
            match crate::studio_presets::apply(&mut app.project, &file, options, dress) {
                Ok(applied) => {
                    app.camera.adopt(app.project.camera());
                    app.submit();
                    let mut out = Object::new();
                    out.set("applied", file.preset.id.clone())
                        .set("views", applied.views as f64)
                        .set("lights", applied.lights as f64)
                        .set("cameras", applied.cameras as f64)
                        .set("scale", applied.scale)
                        .set("dressed", applied.dressed as f64)
                        .set("note", "one undo step; the first view is applied - studio.apply or queue studio=<name> for the others");
                    Reply::Value(out.build())
                }
                Err(e) => Reply::Failed(e),
            }
        }
        "save" => {
            let thumbnail = crate::studio_presets::thumbnail_png(&app.image);
            match crate::studio_presets::save(&app.project, &shared::save_as(request), thumbnail.as_deref()) {
                Ok(file) => {
                    app.library.reload_studios();
                    Reply::Value(shared::describe(&file, false))
                }
                Err(e) => Reply::Failed(e),
            }
        }
        "delete" => {
            let result = shared::named(request, "delete").and_then(|file| presets::delete(&file));
            app.library.reload_studios();
            done(result)
        }
        "install" => match path("the package's or the folder's").and_then(|from| presets::install(&from, &crate::library_tab::user_library_path())) {
            Ok(installed) => {
                app.library.reload();
                app.library.reload_studios();
                let mut out = Object::new();
                out.set("installed", installed.preset.preset.id.clone())
                    .set("path", installed.preset.path.display().to_string())
                    .set("materials", Value::Array(installed.materials.into_iter().map(Value::String).collect()));
                Reply::Value(out.build())
            }
            Err(e) => Reply::Failed(e),
        },
        "export" => done(path("the package's").and_then(|to| shared::named(request, "export").and_then(|file| presets::export(&file, &to)))),
        "render_all" => {
            if app.project.document().studios.is_empty() {
                return Reply::Failed("there are no Studios; apply a studio preset first".into());
            }
            let ids = app.queue_studios(None);
            let mut out = Object::new();
            out.set("jobs", ids.iter().map(|&id| Value::from(id as f64)).collect::<Vec<_>>()).set("count", ids.len() as f64);
            Reply::Value(out.build())
        }
        other => Reply::Failed(format!("`{other}` is not a studio preset action")),
    }
}

/// A material of the caller's own, from a recipe: made, changed, read back,
/// shown on a ball or kept in the library (`tequila_scripting::material`).
fn material(app: &mut TequilaApp, request: &tequila_scripting::protocol::MaterialRequest) -> Reply {
    let selection: Vec<NodeId> = app.project.selection.nodes().collect();
    match tequila_scripting::material::material_request(request, app.project.document(), &selection, &crate::library_tab::user_library_path()) {
        Ok(outcome) => {
            if request.action == "save" {
                app.library.reload();
            }
            carry_out(app, outcome)
        }
        Err(error) => Reply::Failed(error),
    }
}

/// Plugins made and driven by a script or an AI app: written into the
/// user's plugins folder, listed, run, their logs read. Turning one with code
/// on is asked of the person in the Plugins window, never done here.
fn plugin(app: &mut TequilaApp, request: &tequila_scripting::protocol::PluginRequest) -> Reply {
    use crate::plugins::Origin;
    let id = request.plugin.clone().unwrap_or_default();
    let strings = |items: &mut dyn Iterator<Item = String>| Value::Array(items.map(Value::String).collect());
    match request.action.as_str() {
        "guide" => Reply::Value(crate::plugins::guide()),
        "list" => {
            let mut list = Vec::new();
            for plugin in &app.plugins.plugins {
                let id = &plugin.manifest.id;
                let mut entry = Object::new();
                entry
                    .set("id", id.as_str())
                    .set("name", plugin.manifest.name.as_str())
                    .set("version", plugin.manifest.version.as_str())
                    .set("description", plugin.manifest.description.as_str())
                    .set("origin", if plugin.origin == Origin::Bundled { "example" } else { "user" })
                    .set("enabled", app.plugins.is_enabled(id))
                    .set("runs_code", plugin.runs_code())
                    .set("running", app.plugins.hosts.contains_key(id))
                    .set("commands", strings(&mut plugin.manifest.commands.iter().map(|c| c.id.clone())))
                    .set("panels", strings(&mut plugin.manifest.panels.iter().map(|p| p.id.clone())))
                    .set("events", strings(&mut plugin.manifest.events.iter().cloned()))
                    .set("folder", plugin.folder.display().to_string());
                if let Some(failure) = app.plugins.failures.get(id) {
                    entry.set("failure", failure.as_str());
                }
                list.push(entry.build());
            }
            let mut out = Object::new();
            out.set("plugins", Value::Array(list))
                .set("problems", strings(&mut app.plugins.problems.iter().cloned()))
                .set("python", crate::plugins::python().is_some())
                .set("folder", app.plugins.user_folder().display().to_string());
            Reply::Value(out.build())
        }
        "write" => {
            // An existing user plugin is written where it is; a new one, or
            // one of the examples made the person's own, in a folder named
            // for its id.
            let existing = app.plugins.get(&id).filter(|p| p.origin == Origin::User).map(|p| p.folder.clone());
            let folder = existing.clone().unwrap_or_else(|| app.plugins.user_folder().join(&id));
            match crate::plugins::write_files(&folder, &id, &request.files) {
                Ok(folder) => {
                    app.plugins.reload();
                    // A plugin that was not the person's own before - new,
                    // or an example taken over - is not trusted for being
                    // written: they turn it on, and are asked. Nor is one
                    // whose code the asker wants the person to look at again.
                    let runs_code = app.plugins.get(&id).is_some_and(|p| p.runs_code());
                    let was_on = app.plugins.is_enabled(&id);
                    if runs_code && was_on && (existing.is_none() || request.ask) {
                        let _ = app.plugins.set_enabled(&id, false);
                        if request.ask {
                            app.plugins_window.ask_to_enable(&id);
                        }
                    }
                    let restarted = app.plugins.hosts.contains_key(&id) || app.plugins.failures.contains_key(&id);
                    if restarted {
                        app.plugins.restart(&id);
                    }
                    let mut out = Object::new();
                    out.set("id", id.as_str())
                        .set("folder", folder.display().to_string())
                        .set("written", request.files.len() as f64)
                        .set("enabled", app.plugins.is_enabled(&id))
                        .set("restarted", restarted);
                    Reply::Value(out.build())
                }
                Err(error) => Reply::Failed(error),
            }
        }
        _ if app.plugins.get(&id).is_none() => Reply::Failed(format!("no plugin `{id}`; `plugin list` names them")),
        "log" => {
            let wanted = request.lines.unwrap_or(40).max(1);
            let log = app.plugins.logs.get(&id);
            let lines: Vec<String> = log.map(|l| l.iter().skip(l.len().saturating_sub(wanted)).cloned().collect()).unwrap_or_default();
            let mut out = Object::new();
            out.set("id", id.as_str())
                .set("lines", strings(&mut lines.into_iter()))
                .set("total", app.plugins.logged.get(&id).copied().unwrap_or(0) as f64)
                .set("running", app.plugins.hosts.contains_key(&id));
            if let Some(failure) = app.plugins.failures.get(&id) {
                out.set("failure", failure.as_str());
            }
            Reply::Value(out.build())
        }
        "run" => {
            let command = request.command.clone().unwrap_or_default();
            let known = app.plugins.get(&id).is_some_and(|p| p.manifest.commands.iter().any(|c| c.id == command) || p.manifest.panels.iter().any(|panel| panel.fields.iter().any(|f| matches!(&f.kind, crate::plugins::FieldKind::Button { command: c, .. } if *c == command))));
            if !known {
                return Reply::Failed(format!("`{id}` has no command `{command}`"));
            }
            if !app.plugins.is_enabled(&id) {
                return Reply::Failed(format!("`{id}` is off; `plugin enable` asks the person to turn it on"));
            }
            let panel = app.plugins.get(&id).and_then(|p| p.manifest.commands.iter().find(|c| c.id == command)).and_then(|c| c.panel.clone());
            match app.plugins.run(&id, &command, &request.values) {
                Ok(()) => {
                    let mut out = Object::new();
                    match panel {
                        Some(panel) => out.set("opened_panel", panel),
                        None => out.set("sent", true).set("note", "it runs on its own; `plugin log` shows what it printed, `finished` when done, and any traceback"),
                    };
                    Reply::Value(out.build())
                }
                Err(error) => Reply::Failed(error),
            }
        }
        "enable" => {
            let mut out = Object::new();
            if app.plugins.is_enabled(&id) {
                out.set("enabled", true);
            } else if app.plugins.get(&id).is_some_and(|p| p.runs_code()) {
                app.plugins_window.ask_to_enable(&id);
                out.set("enabled", false)
                    .set("asked", true)
                    .set("note", "it runs Python, so Tequila is asking the person to turn it on; `plugin list` shows when they have");
            } else {
                if let Err(error) = app.plugins.set_enabled(&id, true) {
                    return Reply::Failed(error);
                }
                out.set("enabled", true);
            }
            Reply::Value(out.build())
        }
        "restart" => {
            app.plugins.restart(&id);
            Reply::Done
        }
        other => Reply::Failed(format!("`{other}` is not a plugin action")),
    }
}

/// Commands as one undo step, all or none, then the answer.
fn carry_out(app: &mut TequilaApp, outcome: tequila_scripting::document::LibraryOutcome) -> Reply {
    use tequila_scripting::document::LibraryOutcome;
    match outcome {
        LibraryOutcome::Commands(commands, value) => {
            app.project.begin_group();
            for command in commands {
                if let Err(error) = app.project.apply(command) {
                    app.project.abandon_group();
                    return Reply::Failed(error.to_string());
                }
            }
            app.project.end_group();
            app.project.end_interaction();
            Reply::Value(value)
        }
        LibraryOutcome::Done(value) => Reply::Value(value),
    }
}

fn texture_set(app: &mut TequilaApp, path: &str, nodes: &[u64]) -> Reply {
    let named: Vec<NodeId> = if nodes.is_empty() {
        app.project.selection.nodes().collect()
    } else {
        nodes.iter().map(|&n| NodeId(n)).collect()
    };
    // A node that is not there is still reported by the load; a group
    // stands for its parts.
    let targets = match named.iter().find(|n| app.project.document().tree.get(**n).is_none()) {
        Some(_) => named,
        None => tequila_scene::apply::parts_of(app.project.document(), named),
    };
    let loaded = match crate::texture_pack::load(&mut app.project, std::path::Path::new(path), targets) {
        Ok(loaded) => loaded,
        Err(error) => return Reply::Failed(error),
    };
    let report = &loaded.report;
    let materials = report
        .materials
        .iter()
        .zip(&report.maps)
        .map(|((key, name), maps)| {
            let mut entry = Object::new();
            entry.set("id", key.0 as f64).set("name", name.clone()).set(
                "maps",
                Value::Array(
                    maps.iter()
                        .map(|(slot, file)| {
                            let mut map = Object::new();
                            map.set("slot", slot.label()).set("file", file.clone());
                            map.build()
                        })
                        .collect(),
                ),
            );
            entry.build()
        })
        .collect();
    let mut out = Object::new();
    out.set("materials", Value::Array(materials))
        .set("assigned_to", loaded.assigned.len() as f64)
        .set("notes", Value::Array(report.notes.iter().map(|n| Value::String(n.clone())).collect()));
    Reply::Value(out.build())
}

/// Read a model and put it in the document.
///
/// The importer produces a `scene.import` command and this applies it through
/// the bus like any other, so a scripted import is undoable, journalled, and
/// indistinguishable from one done through the File menu.
fn import(app: &mut TequilaApp, path: &str, parent: Option<u64>) -> Reply {
    let path = std::path::PathBuf::from(path);
    if !path.is_file() {
        return Reply::Failed(format!("{} is not a file", path.display()));
    }
    let parent = parent.map(NodeId);
    let model = match tequila_import::read_file(&path) {
        Ok(model) => model,
        Err(error) => return Reply::Failed(error.to_string()),
    };
    // As the File menu imports: the CAD file's bytes kept, so the model
    // divides again without its file, and a guessed unit applied in the
    // same undo step.
    // A model imported into the untouched starter scene takes the starter
    // sphere's place, in one undo step, as the File menu's does.
    let starter = if parent.is_none() { app.untouched_starter() } else { None };
    if let Some(demo) = &starter {
        app.project.begin_group();
        let _ = app.project.apply(tequila_scene::Command::DeleteNodes { nodes: demo.clone() });
    }
    let before = app.node_ids();
    let imported = app.project.import_read_under(model, parent);
    if starter.is_some() {
        if imported.is_ok() {
            app.project.end_group();
        } else {
            app.project.abandon_group();
        }
    }
    let report = match imported {
        Ok((report, _)) => report,
        Err(error) => return Reply::Failed(error),
    };
    app.plugin_imported(&path, &before);

    let mut out = Object::new();
    out.set("path", path.display().to_string())
        .set("nodes", report.nodes as f64)
        .set("meshes", report.meshes_kept as f64)
        .set("triangles", report.triangles as f64)
        .set("textures", report.textures as f64)
        // Which unit the geometry was taken as, and whether that was read
        // from the file or guessed from its size. A model that came in at a
        // thousand times its intended scale is the most common import
        // surprise there is, and this is where a script can notice.
        .set("unit", format!("{:?}", report.source_unit))
        .set("unit_was_guessed", report.unit_was_guessed);
    // The warnings are the whole value of an import report. A script that
    // cannot see "this file wanted a texture I could not read" will ship the
    // render anyway.
    out.set(
        "warnings",
        Value::Array(
            report
                .warnings
                .iter()
                .map(|w| Value::String(w.clone()))
                .collect(),
        ),
    );
    Reply::Value(out.build())
}

/// Write the project out.
fn save(app: &mut TequilaApp, path: Option<&str>) -> Reply {
    let target = match path {
        Some(path) => std::path::PathBuf::from(path),
        None => match app.project.path.clone() {
            Some(path) => path,
            None => {
                return Reply::Failed(
                    "this project has never been saved, so a path is needed".into(),
                )
            }
        },
    };
    match app.project.save(&target) {
        Ok(()) => {
            let mut out = Object::new();
            out.set("path", absolute(&target));
            Reply::Value(out.build())
        }
        Err(error) => Reply::Failed(error.to_string()),
    }
}

/// A path a client can actually find again.
///
/// A relative path in a request is resolved against the *application's*
/// working directory, which is rarely the client's — so a reply that echoed
/// what it was given would send the caller looking in the wrong place. Asking
/// where the file really went costs nothing and removes a whole class of
/// "it said it worked and there is nothing there".
fn absolute(path: &std::path::Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

/// Render the document as it stands, to a file.
///
/// Synchronous, and it blocks the frame loop for as long as it takes. That is
/// the right trade for a scripted render: the client asked for a finished
/// picture and has nothing to do until it exists, and a progress protocol for
/// an operation nobody is watching would be machinery for its own sake.
/// A wireframe of the view to a PNG, as Render wireframe draws one: the
/// document's Wireframe settings, at the size asked for.
fn wireframe_to_file(app: &mut TequilaApp, path: &str, width: usize, height: usize) -> Result<Value, String> {
    if !(1..=16384).contains(&width) || !(1..=16384).contains(&height) {
        return Err("width and height must be between 1 and 16384 pixels".into());
    }
    let started = std::time::Instant::now();
    let settings = tequila_scene::render::RenderSettings { width, height, ..app.project.document().render.clone() };
    let scene = app.project.scene();
    let output = crate::batch::wireframe_output(&scene, &app.camera.camera, &settings)?;
    output
        .write(std::path::Path::new(path), tequila_format::output::ImageFormat::Png(tequila_format::psd::Depth::Eight))
        .map_err(|e| e.to_string())?;
    let mut out = Object::new();
    out.set("path", absolute(std::path::Path::new(path)))
        .set("width", width as f64)
        .set("height", height as f64)
        .set("wireframe", true)
        .set("edges", settings.wireframe.edges.key())
        .set("seconds", started.elapsed().as_secs_f64());
    Ok(out.build())
}

fn render_to_file(
    app: &mut TequilaApp,
    path: &str,
    width: usize,
    height: usize,
    samples: u32,
) -> Result<Value, String> {
    use tequila_render_core::framebuffer::Framebuffer;
    use tequila_render_core::scheduler::ProgressiveSchedule;
    use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};

    // This render runs on the window's own thread and holds a full-float
    // framebuffer, so it is bounded: 40000 by 40000 asked for over a hundred
    // gigabytes and took the application down, and a billion samples froze
    // it for hours. `queue` is the way to render big or long.
    if !(1..=16384).contains(&width) || !(1..=16384).contains(&height) {
        return Err("width and height must be between 1 and 16384 pixels".into());
    }
    if width * height > 40_000_000 {
        return Err(format!("{width} × {height} is more than 40 megapixels; use `queue` for a render that big"));
    }
    if !(1..=16_384).contains(&samples) {
        return Err("samples must be between 1 and 16384; use `queue` for more".into());
    }
    let device = Device::shared().map_err(|e| e.to_string())?;
    let scene = app.project.scene();
    let intersector =
        EmbreeIntersector::build(&device, (*scene).clone()).map_err(|e| e.to_string())?;

    let mut framebuffer = Framebuffer::new(width, height);
    let settings = &app.project.document().render;
    let context = RenderContext {
        camera: app.camera.camera,
        config: app.project.integrator_config(),
        reject_fireflies: settings.reject_fireflies,
        clamp_sample: settings.clamp_indirect,
        ..RenderContext::default()
    };
    let mut schedule = ProgressiveSchedule::default();
    schedule.update(60_000);
    let started = std::time::Instant::now();
    render_pass(
        &intersector,
        &mut framebuffer,
        &context,
        &schedule,
        samples,
    );

    // Scripted exports use the same output-quality settings as queued renders.
    // Filter a copy, leaving the accumulation and its guides untouched.
    // An image with alpha carries its foreground alone; see `Pixel::mean_foreground`.
    let foreground = settings.alpha_mode.transparent_background();
    let silhouette = settings.alpha_mode == tequila_scene::render::AlphaMode::Silhouette;
    let denoised = if settings.denoise {
        let buffers = framebuffer.to_denoiser_buffers_with(foreground);
        Some(tequila_denoise::Denoiser::shared()
            .map_err(|e| format!("Could not initialize the requested denoiser: {e}"))?
            .denoise(width, height, &buffers.color, &buffers.albedo, &buffers.normal,
                tequila_denoise::DenoiseSettings {
                    quality: tequila_denoise::Quality::High,
                    strength: settings.denoise_strength,
                    prefilter_guides: settings.prefilter_guides,
                })
            .map_err(|e| format!("Denoising failed: {e}"))?)
    } else { None };

    let mut color = Vec::with_capacity(width * height);
    let mut alpha = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            color.push(match &denoised {
                Some(image) => {
                    let i = (y * width + x) * 3;
                    tequila_core::spectrum::Spectrum::from_rendering_rgb(Vec3 {
                        x: image.data[i], y: image.data[i + 1], z: image.data[i + 2],
                    })
                }
                None if foreground => framebuffer.pixel(x, y).mean_foreground(),
                None => framebuffer.pixel(x, y).mean(),
            });
            alpha.push(framebuffer.pixel(x, y).alpha());
        }
    }

    let (color, alpha) = if silhouette {
        tequila_render_cpu::Silhouette::trace(&intersector, &context.camera, width, height, 24, context.scene_seed).cut(&color)
    } else {
        (color, alpha)
    };

    // The document's own look, not a neutral one: a script checking a material
    // should see what the person looking at the viewport sees.
    let style = app.project.document().image_style;
    let display = app.project.display_pipeline();
    let output = tequila_format::output::RenderOutput {
        width,
        height,
        color,
        alpha,
        layers: Vec::new(),
        style,
        display,
        // The document's choice, as a render from the Render window makes.
        alpha_mode: crate::batch::output_alpha(&app.project.document().render),
    };
    output
        .write(
            std::path::Path::new(path),
            tequila_format::output::ImageFormat::Png(tequila_format::psd::Depth::Eight),
        )
        .map_err(|e| e.to_string())?;

    let mut out = Object::new();
    out.set("path", absolute(std::path::Path::new(path)))
        .set("width", width as f64)
        .set("height", height as f64)
        .set("samples", samples as f64)
        .set("denoised", denoised.is_some())
        .set("seconds", started.elapsed().as_secs_f64());
    Ok(out.build())
}

impl TequilaApp {
    /// Hand over the window as a PNG, once egui has one to give.
    ///
    /// The capture is an event rather than a return value, so this runs on
    /// every frame and does its work on whichever frame the image turns up.
    /// The client is already waiting on the file rather than on the reply,
    /// which is what the `pending` in that reply was telling it.
    pub(crate) fn serve_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.pending_screenshot.clone() else {
            self.screenshot_requested = false;
            return;
        };
        // A window of its own is captured as it draws itself.
        if self.screenshot_window.is_some() {
            self.settle_window_capture();
            return;
        }
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        match image {
            Some(image) => {
                self.pending_screenshot = None;
                self.screenshot_requested = false;
                // Just the viewport when asked: what a script inspecting a
                // material wants to see is the picture, not the panels.
                let image = if std::mem::take(&mut self.screenshot_viewport)
                    && self.viewport_rect.is_positive()
                {
                    std::sync::Arc::new(image.region(&self.viewport_rect, Some(ctx.pixels_per_point())))
                } else {
                    image
                };
                if let Err(error) = write_png(&path, &image) {
                    self.project.status =
                        format!("The screenshot could not be written: {error}");
                }
            }
            None => {
                // A minimized window has no pixels to read back: egui would
                // read a framebuffer zero pixels wide, and that panics. So
                // the screenshot waits for the window to be restored.
                let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false) || !i.screen_rect().is_positive());
                if minimized {
                    self.project.status = "A screenshot is waiting for Tequila's window to be restored".into();
                } else if !self.screenshot_requested {
                    self.screenshot_requested = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                }
            }
        }
    }
}

impl TequilaApp {
    /// Finish with a capture of one of the windows once it is written, or
    /// once its window has closed and it never will be.
    pub(crate) fn settle_window_capture(&mut self) {
        let Some(kind) = self.screenshot_window else { return };
        let finished = match crate::windows::capture_done() {
            Some(result) => Some(result),
            None if !self.window_open(kind) => {
                crate::windows::cancel_capture();
                Some(Err(format!("the {} window closed first", kind.key())))
            }
            None => None,
        };
        if let Some(result) = finished {
            self.pending_screenshot = None;
            self.screenshot_window = None;
            if let Err(error) = result {
                self.project.status = format!("The screenshot could not be written: {error}");
            }
        }
    }
}

pub(crate) fn write_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(file, image.width() as u32, image.height() as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_array())
        .collect();
    writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}

#[cfg(test)]
mod scripting_tests {
    use super::*;

    #[test]
    fn discovery_examples_and_material_definitions_are_machine_readable() {
        let api = tequila_scripting::document::api(OPERATIONS, QUERIES, LIMITS);
        let wire = tequila_format::json_write::to_string(&api);
        assert!(tequila_format::json::parse(wire.as_bytes()).is_ok());
        assert!(!api.get("material_parameters").unwrap().as_array().is_empty());
        for example in api.get("examples").unwrap().as_array() {
            tequila_scripting::protocol::parse_request(&tequila_format::json_write::to_string(example.get("request").unwrap())).unwrap();
        }
        let doc = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
        let description = tequila_format::scene_description(&doc);
        let parsed = tequila_format::json::parse(description.as_bytes()).unwrap();
        for key in ["tree", "units", "environment", "lighting", "cameras", "render"] { assert!(parsed.get(key).is_some()); }
        let result = materials(&doc);
        for material in result.get("materials").unwrap().as_array() {
            assert!(material.get("definition").unwrap().get("graph").is_some() || material.get("definition").unwrap().get("base_color").is_some());
        }
    }
}

/// The handler, end to end: real requests, parsed by the real parser, answered
/// by the real handler, on a real application — built without a journal,
/// a socket or a window, so nothing here touches the machine it runs on.
#[cfg(test)]
mod handler_tests {
    use super::*;
    use crate::scripting_tab::Outcome;
    use tequila_scripting::protocol::parse_request;

    fn ask(app: &mut TequilaApp, line: &str) -> Reply {
        let request = parse_request(line).unwrap_or_else(|e| panic!("{line}: {e}"));
        handle(app, &request)
    }

    #[track_caller]
    fn value(app: &mut TequilaApp, line: &str) -> Value {
        match ask(app, line) {
            Reply::Value(value) => value,
            Reply::Done => Value::Null,
            Reply::Failed(message) => panic!("{line} failed: {message}"),
        }
    }

    #[track_caller]
    fn failure(app: &mut TequilaApp, line: &str) -> String {
        match ask(app, line) {
            Reply::Failed(message) => message,
            other => panic!("{line} should have failed, got {other:?}"),
        }
    }

    /// Two node ids from the demo document. Any nodes: renaming is what the
    /// tests do with them, and a group renames like a part.
    fn two_parts(app: &mut TequilaApp) -> (u64, u64) {
        let nodes = value(app, r#"{"op":"query","what":"nodes"}"#);
        let ids: Vec<u64> = nodes
            .get("nodes")
            .unwrap()
            .as_array()
            .iter()
            .map(|n| n.get("id").unwrap().as_f64().unwrap() as u64)
            .collect();
        assert!(ids.len() >= 2, "the demo scene has at least two nodes: {ids:?}");
        (ids[0], ids[1])
    }

    fn name_of(app: &TequilaApp, id: u64) -> String {
        app.project.document().tree.get(NodeId(id)).unwrap().name.clone()
    }

    fn rename(id: u64, name: &str) -> String {
        format!(r#"{{"op":"apply","command":{{"command":"scene.rename","node":{id},"name":"{name}"}}}}"#)
    }

    /// A script imports a model, works on it, and reloads it after the file
    /// changed: the part it worked on keeps its new name, and the reply says
    /// what was added and what was hidden.
    /// A scripted import keeps a CAD file's own bytes with the project, as
    /// the File menu's does, so the model can be divided again without the
    /// file. It named the source on the model and kept nothing.
    #[test]
    fn a_scripted_cad_import_keeps_its_source() {
        let mut app = TequilaApp::for_tests();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../crates/tequila-import/tests/data/two_bodies.step").replace('\\', "/");
        let reply = ask(&mut app, &format!(r#"{{"op":"import","path":"{path}"}}"#));
        assert!(!matches!(reply, Reply::Failed(_)), "{reply:?}");
        let document = app.project.document();
        let named: Vec<&String> = document.tree.iter().filter_map(|n| n.metadata.get(tequila_scene::document::BREP)).collect();
        assert!(!named.is_empty(), "the model names no source");
        assert!(named.iter().all(|id| document.sources.get(id).is_some()), "the model names a source the project does not keep");
    }

    /// A file that does not say its unit comes in at the unit its size
    /// suggests, from a script as from the File menu: 120 by 80 is
    /// millimetres, so 0.12 m wide, not 120 m.
    #[test]
    fn a_scripted_import_takes_the_unit_it_guesses() {
        let mut app = TequilaApp::for_tests();
        let directory = std::env::temp_dir().join(format!("tequila-scripted-units-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("plate.obj");
        std::fs::write(&file, "v 0 0 0\nv 120 0 0\nv 120 80 0\nv 0 80 0\nf 1 2 3 4\n").unwrap();
        let path = file.display().to_string().replace('\\', "/");
        let reply = ask(&mut app, &format!(r#"{{"op":"import","path":"{path}"}}"#));
        assert!(!matches!(reply, Reply::Failed(_)), "{reply:?}");
        let document = app.project.document();
        assert!(!document.tree.iter().any(|n| n.name == "Demo"), "the starter sphere stayed beside the model");
        let top = document.tree.iter().find(|n| n.name.starts_with("plate")).expect("the model").id;
        let bounds = document.tree.bounds(top, &|m| document.geometry.get(m).map(|m| m.bounds()));
        let width = bounds.max.x - bounds.min.x;
        assert!((width - 0.12).abs() < 1e-3, "120 units came in {width} wide");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_scripted_reimport_keeps_the_work() {
        let directory = std::env::temp_dir().join(format!("tequila-channel-reimport-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("bracket.obj");
        let obj = |parts: &[(&str, f32)]| {
            let mut text = String::new();
            for (i, (name, s)) in parts.iter().enumerate() {
                let b = i * 3 + 1;
                text.push_str(&format!("o {name}\nv 0 0 0\nv {s} 0 0\nv 0 {s} 0\nf {} {} {}\n", b, b + 1, b + 2));
            }
            text
        };
        std::fs::write(&file, obj(&[("Housing", 1.0), ("Clip", 0.2)])).unwrap();
        let path = file.display().to_string().replace('\\', "/");

        let mut app = TequilaApp::for_tests();
        value(&mut app, &format!(r#"{{"op":"import","path":"{path}"}}"#));
        let housing = app.project.document().tree.iter().find(|n| n.name == "Housing").unwrap().id;
        value(&mut app, &rename(housing.0, "Case"));

        std::fs::write(&file, obj(&[("Housing", 2.0), ("Lid", 1.0)])).unwrap();
        let answer = value(&mut app, &format!(r#"{{"op":"reimport","node":{}}}"#, housing.0));
        assert!(answer.get("matched").unwrap().as_f64().unwrap() >= 1.0);
        let added: Vec<&str> = answer.get("added").unwrap().as_array().iter().filter_map(|v| v.as_str()).collect();
        assert!(added.contains(&"Lid"), "{added:?}");
        let removed = answer.get("removed").unwrap().as_array();
        assert_eq!(removed.len(), 1);
        let part = app.project.document().tree.get(housing).expect("the housing kept its id");
        assert_eq!(part.name, "Case", "the rename was lost");

        assert!(failure(&mut app, r#"{"op":"reimport","node":99999}"#).contains("no node"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn every_query_answers() {
        let mut app = TequilaApp::for_tests();
        for what in ["api", "context", "scene", "nodes", "materials", "selection", "camera", "commands", "ui", "jobs", "history"] {
            let answer = value(&mut app, &format!(r#"{{"op":"query","what":"{what}"}}"#));
            assert!(!matches!(answer, Value::Null), "{what} answered nothing");
        }
    }

    /// The catalog a script sees is the whole catalog, and every name in it
    /// is one the parser accepts as a command.
    #[test]
    fn the_commands_query_lists_the_whole_catalog() {
        let mut app = TequilaApp::for_tests();
        let answer = value(&mut app, r#"{"op":"query","what":"commands"}"#);
        let names = answer.get("commands").unwrap().as_array();
        assert_eq!(names.len(), tequila_scene::Command::CATALOG.len());
        for name in ["scene.split_parts", "material.set_mapping", "environment.set_sky"] {
            assert!(names.iter().any(|n| n.as_str() == Some(name)), "{name} is missing");
        }
    }

    /// The api carries an example of every command, and each is one apply
    /// accepts. Five of forty-nine had one; a script or the Assistant had to
    /// guess the fields of the rest.
    #[test]
    fn the_api_has_an_example_of_every_command() {
        let mut app = TequilaApp::for_tests();
        let api = value(&mut app, r#"{"op":"query","what":"api"}"#);
        let examples = api.get("command_examples").expect("command examples");
        for name in tequila_scene::Command::CATALOG {
            let example = examples.get(name).unwrap_or_else(|| panic!("no example of `{name}`"));
            let line = tequila_format::json_write::to_string(example);
            tequila_format::journal::decode(&line).unwrap_or_else(|e| panic!("`{name}`'s example does not parse: {e}"));
        }
    }

    /// The api lists every pattern, and each one as listed is exactly what
    /// material.set_procedural takes: sent straight back, it applies and
    /// lands on the material unchanged.
    #[test]
    fn the_api_lists_every_procedural_in_the_shape_the_command_takes() {
        let mut app = TequilaApp::for_tests();
        let api = value(&mut app, r#"{"op":"query","what":"api"}"#);
        let listed = api.get("procedurals").expect("the api lists procedurals").as_array().to_vec();
        let catalog = tequila_texture::procedural::Procedural::catalog();
        assert_eq!(listed.len(), catalog.len());
        let material = app.project.document().materials.iter().next().expect("the demo has a material").0;
        for (entry, expected) in listed.iter().zip(catalog) {
            let request = format!(
                r#"{{"op":"apply","command":{{"command":"material.set_procedural","material":{},"param":"specular_roughness","seed":7,"procedural":{}}}}}"#,
                material.0,
                tequila_format::json_write::to_string(entry)
            );
            value(&mut app, &request);
            let graph = &app.project.document().materials.get(material).unwrap().graph;
            let (applied, seed) = tequila_scene::command::procedural_of(graph, tequila_materials::material_type::Param::SpecularRoughness)
                .unwrap_or_else(|| panic!("{} did not land", expected.label()));
            assert_eq!((applied, seed), (expected, 7), "{} came back changed", expected.label());
        }
    }

    #[test]
    fn a_batch_is_one_undo_step() {
        let mut app = TequilaApp::for_tests();
        let (a, b) = two_parts(&mut app);
        let (old_a, old_b) = (name_of(&app, a), name_of(&app, b));

        let answer = value(&mut app, &format!(r#"{{"op":"batch","ops":[{},{}]}}"#, rename(a, "First"), rename(b, "Second")));
        assert_eq!(answer.get("count").and_then(|c| c.as_f64()), Some(2.0));
        assert_eq!((name_of(&app, a), name_of(&app, b)), ("First".into(), "Second".into()));

        let undone = value(&mut app, r#"{"op":"undo"}"#);
        assert_eq!(undone.get("commands").and_then(|c| c.as_f64()), Some(2.0), "the reply says how much came back");
        assert_eq!((name_of(&app, a), name_of(&app, b)), (old_a, old_b), "one undo took the whole batch back");
    }

    /// The case atomic exists for: the batch fails half-way, and the half
    /// that worked must not be left in the scene.
    #[test]
    fn a_failed_batch_leaves_the_scene_as_it_was() {
        let mut app = TequilaApp::for_tests();
        let (a, _) = two_parts(&mut app);
        let before = name_of(&app, a);
        let undo_before = app.project.undo_step();

        let message = failure(&mut app, &format!(r#"{{"op":"batch","ops":[{},{{"op":"select","nodes":[999999]}}]}}"#, rename(a, "Half done")));
        assert!(message.contains("operation 2 of 2"), "{message}");
        assert!(message.contains("taken back"), "{message}");
        assert_eq!(name_of(&app, a), before);
        assert_eq!(app.project.undo_step(), undo_before, "nothing new to undo");
        assert!(!app.project.can_redo(), "and nothing to redo into");
    }

    #[test]
    fn a_non_atomic_batch_keeps_what_worked() {
        let mut app = TequilaApp::for_tests();
        let (a, _) = two_parts(&mut app);
        let message = failure(&mut app, &format!(r#"{{"op":"batch","atomic":false,"ops":[{},{{"op":"select","nodes":[999999]}}]}}"#, rename(a, "Kept")));
        assert!(message.contains("kept"), "{message}");
        assert_eq!(name_of(&app, a), "Kept");
    }

    #[test]
    fn undo_and_quit_are_refused_inside_a_batch() {
        let mut app = TequilaApp::for_tests();
        let (a, _) = two_parts(&mut app);
        let before = name_of(&app, a);
        let message = failure(&mut app, &format!(r#"{{"op":"batch","ops":[{},{{"op":"undo"}}]}}"#, rename(a, "X")));
        assert!(message.contains("cannot be part of a batch"), "{message}");
        assert_eq!(name_of(&app, a), before);
        assert!(failure(&mut app, r#"{"op":"batch","ops":[{"op":"quit","discard":true}]}"#).contains("quit"));
        assert!(!app.quit_requested);
    }

    /// A script never closes the window on someone's work by default.
    #[test]
    fn quit_refuses_unsaved_work_unless_told() {
        let mut app = TequilaApp::for_tests();
        let (a, _) = two_parts(&mut app);
        value(&mut app, &rename(a, "Unsaved"));

        let message = failure(&mut app, r#"{"op":"quit"}"#);
        assert!(message.contains("unsaved"), "{message}");
        assert!(!app.quit_requested);

        // Nowhere to save to, so save:true must fail and not close either.
        assert!(failure(&mut app, r#"{"op":"quit","save":true}"#).contains("save"));
        assert!(!app.quit_requested);

        value(&mut app, r#"{"op":"quit","discard":true}"#);
        assert!(app.quit_requested);
    }

    #[test]
    fn ui_switches_panels_and_refuses_bad_names_whole() {
        let mut app = TequilaApp::for_tests();
        let state = value(&mut app, r#"{"op":"ui","tab":"material","console":true,"render_window":true,"viewport":"rendered"}"#);
        assert_eq!(state.get("tab").and_then(|t| t.as_str()), Some("material"));
        assert!(app.console.open && app.render_window.open && app.show_project);
        assert_eq!(app.shading, crate::orchestrator::Shading::Rendered);

        // One bad field changes nothing, not everything before it.
        let message = failure(&mut app, r#"{"op":"ui","console":false,"tab":"nonsense"}"#);
        assert!(message.contains("scene"), "the refusal lists the real tabs: {message}");
        assert!(app.console.open, "the console was not closed by a refused request");
    }

    /// A screenshot asked of a minimized window waits for it to come back,
    /// rather than reading a framebuffer with no pixels, which panics.
    #[test]
    fn a_screenshot_waits_while_the_window_is_minimized() {
        let mut app = TequilaApp::for_tests();
        let path = std::env::temp_dir().join(format!("tequila-minimized-{}.png", std::process::id()));
        let text = path.display().to_string().replace('\\', "/");
        value(&mut app, &format!(r#"{{"op":"screenshot","path":"{text}"}}"#));
        let ctx = egui::Context::default();
        let asks_for_pixels = |app: &mut TequilaApp, minimized: bool| {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                ..Default::default()
            };
            input.viewports.entry(egui::ViewportId::ROOT).or_default().minimized = Some(minimized);
            let output = ctx.run(input, |ctx| app.serve_screenshot(ctx));
            output
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .is_some_and(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::Screenshot)))
        };
        assert!(!asks_for_pixels(&mut app, true), "a minimized window was asked for its pixels");
        assert!(app.pending_screenshot.is_some(), "the screenshot was given up");
        assert!(asks_for_pixels(&mut app, false), "the restored window was not asked for its pixels");
    }

    #[test]
    fn ui_opens_and_closes_the_graph_editor() {
        let mut app = TequilaApp::for_tests();
        let key = app.project.document().materials.iter().next().expect("the demo has materials").0;
        let state = value(&mut app, &format!(r#"{{"op":"ui","graph":{}}}"#, key.0));
        assert_eq!(state.get("graph").and_then(|g| g.as_f64()), Some(key.0 as f64));
        assert!(app.graph_editor.open);

        assert!(failure(&mut app, r#"{"op":"ui","graph":987654}"#).contains("no material"));
        assert!(app.graph_editor.open, "a refused request leaves the editor as it was");

        let state = value(&mut app, r#"{"op":"ui","graph":false}"#);
        assert!(state.get("graph").is_some_and(|g| matches!(g, Value::Null)));
        assert!(!app.graph_editor.open);
    }

    #[test]
    fn look_moves_the_camera_undoably() {
        let mut app = TequilaApp::for_tests();
        let start = app.camera.camera.position();

        let top = value(&mut app, r#"{"op":"look","view":"top"}"#);
        assert!(top.get("pitch").and_then(|p| p.as_f64()).unwrap() > 80.0, "top looks down");
        assert!((app.camera.camera.position() - start).length() > 1e-3);

        value(&mut app, r#"{"op":"look","from":[0,1,4],"at":[0,1,0],"focal_length":85}"#);
        let camera = value(&mut app, r#"{"op":"query","what":"camera"}"#);
        assert_eq!(camera.get("focal_length").and_then(|f| f.as_f64()), Some(85.0));
        let position = app.camera.camera.position();
        assert!((position.z - 4.0).abs() < 1e-3 && (position.y - 1.0).abs() < 1e-3, "{position:?}");

        value(&mut app, r#"{"op":"undo"}"#);
        value(&mut app, r#"{"op":"undo"}"#);
        assert!((app.camera.camera.position() - start).length() < 1e-3, "two undos, back where it started");

        assert!(failure(&mut app, r#"{"op":"look","from":[0,1,0],"at":[0,1,0]}"#).contains("standing on"));
        assert!(failure(&mut app, r#"{"op":"look","view":"sideways"}"#).contains("front"));
    }

    /// A network render with no farm set up says what is missing, rather
    /// than queuing into nothing.
    #[test]
    fn a_network_render_without_a_farm_says_what_is_missing() {
        let mut app = TequilaApp::for_tests();
        std::env::remove_var("TEQUILA_FARM_TOKEN");
        let message = failure(&mut app, r#"{"op":"queue","network":true}"#);
        assert!(message.contains("token"), "{message}");
    }

    #[test]
    fn queue_adds_a_background_job_without_touching_the_document() {
        let mut app = TequilaApp::for_tests();
        let settings = app.project.render_settings().clone();
        let job = value(&mut app, r#"{"op":"queue","name":"Tiny","width":320,"height":200,"samples":4}"#);
        let id = job.get("job").and_then(|j| j.as_f64()).unwrap();

        let jobs = value(&mut app, r#"{"op":"query","what":"jobs"}"#);
        let listed = jobs.get("jobs").unwrap().as_array();
        let entry = listed.iter().find(|j| j.get("id").and_then(|i| i.as_f64()) == Some(id)).expect("listed");
        assert_eq!(entry.get("state").and_then(|s| s.as_str()), Some("queued"));
        assert_eq!(entry.get("width").and_then(|w| w.as_f64()), Some(320.0));
        assert_eq!(app.project.render_settings(), &settings, "the overrides are the job's, not the document's");

        assert!(failure(&mut app, r#"{"op":"queue","width":4}"#).contains("between"));
        assert!(failure(&mut app, r#"{"op":"queue","camera":99}"#).contains("camera"));
        assert!(failure(&mut app, r#"{"op":"stop"}"#).contains("nothing"));
    }

    /// What a person watching the console sees when an assistant acts.
    #[test]
    fn a_remote_batch_is_spelled_out_in_the_console() {
        let mut app = TequilaApp::for_tests();
        let (a, b) = two_parts(&mut app);
        let mut request = parse_request(&format!(r#"{{"op":"batch","ops":[{},{}]}}"#, rename(a, "P"), rename(b, "Q"))).unwrap();
        request.peer = "127.0.0.1:50000".into();
        assert!(matches!(handle(&mut app, &request), Reply::Value(_)));
        let shown = app.console.text();
        assert!(shown.contains("127.0.0.1:50000  batch of 2"), "{shown}");
        assert_eq!(shown.matches("· scene.rename").count(), 2, "{shown}");
        assert_eq!(app.activity.newest_first().next().map(|e| e.outcome), Some(Outcome::Ok));
    }

    /// A script opens a project and starts a new one: refused over unsaved
    /// work unless it says what to do with it, the project remembered in
    /// File > Open recent, and Help > About opened on request.
    #[test]
    fn a_script_opens_and_starts_projects() {
        let mut app = TequilaApp::for_tests();
        let folder = std::env::temp_dir().join(format!("tequila-channel-open-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("kettle.tequila");
        let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
        document.meta.name = "Kettle".into();
        tequila_format::save(&file, &document, &tequila_format::Preserved::default()).unwrap();
        let path = file.display().to_string().replace('\\', "/");
        let node = app.project.document().tree.iter().next().unwrap().id;
        value(&mut app, &format!(r#"{{"op":"apply","command":{{"command":"scene.rename","node":{},"name":"Unsaved"}}}}"#, node.0));
        assert!(failure(&mut app, &format!(r#"{{"op":"open","path":"{path}"}}"#)).contains("unsaved"));
        value(&mut app, &format!(r#"{{"op":"open","path":"{path}","discard":true}}"#));
        assert_eq!(app.project.document().meta.name, "Kettle");
        assert_eq!(app.project.path.as_deref(), Some(file.as_path()));
        let ui = value(&mut app, r#"{"op":"query","what":"ui"}"#);
        let recent: Vec<&str> = ui.get("recent").unwrap().as_array().iter().filter_map(Value::as_str).collect();
        assert!(recent.first().is_some_and(|r| r.ends_with("kettle.tequila")), "{recent:?}");
        value(&mut app, r#"{"op":"new"}"#);
        assert!(app.project.path.is_none());
        let names: Vec<String> = app.project.document().tree.iter().map(|n| n.name.clone()).collect();
        assert_eq!(names, vec!["Demo".to_string(), "Sphere".to_string()], "a new project is the starter scene");
        value(&mut app, r#"{"op":"ui","about":true}"#);
        assert!(app.show_about);
        assert_eq!(value(&mut app, r#"{"op":"query","what":"ui"}"#).get("about").and_then(Value::as_bool), Some(true));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// An AI app makes a plugin through the channel: it is written, listed,
    /// off; asking to turn it on asks the person, and it does not run until
    /// they say yes. A material pack has no code, and is turned on at once.
    #[test]
    fn a_plugin_is_written_listed_and_asked_for() {
        use tequila_format::json_write::{to_string, Object};
        let root = std::env::temp_dir().join(format!("tequila-channel-plugins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut app = TequilaApp::for_tests();
        // Never the person's own plugins folder.
        app.plugins = crate::plugins::Plugins::load_from(vec![(root.clone(), crate::plugins::Origin::User)], root.join("enabled.json"));
        let write = |id: &str, files: &[(&str, &str)]| {
            let mut map = Object::new();
            for (path, text) in files {
                map.set(path, *text);
            }
            let mut request = Object::new();
            request.set("id", 1.0).set("op", "plugin").set("action", "write").set("plugin", id).set("files", map.build());
            to_string(&request.build())
        };
        let manifest = r#"{"id":"my.hello","name":"Hello","main":"main.py","commands":[{"id":"go"}]}"#;
        let main = "import tequila_plugin as plugin\n\n@plugin.command('go')\ndef go(fs, values):\n    plugin.status('hi')\n";
        let written = value(&mut app, &write("my.hello", &[("plugin.json", manifest), ("main.py", main)]));
        assert_eq!(written.get("enabled").and_then(Value::as_bool), Some(false));
        assert!(root.join("my.hello").join("main.py").is_file());
        assert!(failure(&mut app, &write("my.hello", &[("../x.py", "")])).contains("inside the plugin"));

        let list = value(&mut app, r#"{"op":"plugin","action":"list"}"#);
        let hello = list.get("plugins").unwrap().as_array().iter().find(|p| p.get("id").and_then(Value::as_str) == Some("my.hello")).cloned().expect("listed");
        assert_eq!(hello.get("runs_code").and_then(Value::as_bool), Some(true));
        assert_eq!(hello.get("commands").unwrap().as_array()[0].as_str(), Some("go"));

        assert!(failure(&mut app, r#"{"id":1,"op":"plugin","action":"run","plugin":"my.hello","command":"go"}"#).contains("is off"));
        assert!(failure(&mut app, r#"{"id":1,"op":"plugin","action":"run","plugin":"my.hello","command":"nope"}"#).contains("no command"));
        let asked = value(&mut app, r#"{"id":1,"op":"plugin","action":"enable","plugin":"my.hello"}"#);
        assert_eq!(asked.get("asked").and_then(Value::as_bool), Some(true));
        assert!(!app.plugins.is_enabled("my.hello"), "turned on without the person");
        assert_eq!(app.plugins_window.confirm_enable.as_deref(), Some("my.hello"));
        assert!(app.plugins_window.open);

        let pack = r#"{"id":"my.woods","name":"Woods","materials":"materials"}"#;
        value(&mut app, &write("my.woods", &[("plugin.json", pack)]));
        let enabled = value(&mut app, r#"{"id":1,"op":"plugin","action":"enable","plugin":"my.woods"}"#);
        assert_eq!(enabled.get("enabled").and_then(Value::as_bool), Some(true));
        assert!(app.plugins.is_enabled("my.woods"));

        // On, then changed by the built-in assistant (`ask`): turned off, and
        // the person asked again before the new code runs.
        app.plugins.set_enabled("my.hello", true).unwrap();
        app.plugins_window.confirm_enable = None;
        let mut rewrite = tequila_format::json::parse(write("my.hello", &[("main.py", main)]).as_bytes()).unwrap();
        if let Value::Object(map) = &mut rewrite {
            map.insert("ask".into(), Value::Bool(true));
        }
        let rewritten = value(&mut app, &to_string(&rewrite));
        assert_eq!(rewritten.get("enabled").and_then(Value::as_bool), Some(false));
        assert!(!app.plugins.is_enabled("my.hello"), "changed code was left on without asking");
        assert_eq!(app.plugins_window.confirm_enable.as_deref(), Some("my.hello"));
        // Without `ask` - an AI app the person connected with full access -
        // a plugin that is on stays on, restarting with the new code.
        app.plugins.set_enabled("my.hello", true).unwrap();
        value(&mut app, &write("my.hello", &[("main.py", main)]));
        assert!(app.plugins.is_enabled("my.hello"));

        let log = value(&mut app, r#"{"id":1,"op":"plugin","action":"log","plugin":"my.hello"}"#);
        assert_eq!(log.get("total").and_then(Value::as_f64), Some(0.0));
        assert!(failure(&mut app, r#"{"id":1,"op":"plugin","action":"log","plugin":"my.nobody"}"#).contains("no plugin"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
