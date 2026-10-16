//! The Assistant: ask for what you want in plain language, and an AI you have
//! connected — Claude, OpenAI, a local model, your company's — does it in the
//! open scene.
//!
//! [`tequila_ai`] holds the conversation and talks to the provider; this is
//! the panel, and the part that carries out tool calls. A tool call becomes a
//! scripting request and goes through [`crate::channel::handle`] like any
//! other: validated, undoable, journalled, and written into the console with
//! the assistant's name on it, so a person can watch it work.

use crate::TequilaApp;
use tequila_ai::{session, tools, Call, Key, Kind, Part, Provider, Session, Settings};
use tequila_scripting::protocol::{parse_request, Reply};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

/// The longest side of a screenshot sent to a model. Larger costs tokens
/// without helping it judge a material or a composition.
const SENT_SIZE: usize = 1024;

/// What the panel shows. Separate from the conversation, which holds what the
/// model is sent: the panel shows every picture, the model only the newest.
enum Entry {
    You(String),
    Said(String),
    Step { text: String, ok: bool, detail: String },
    Picture(egui::TextureHandle),
    Problem(String),
    Note(String),
}

/// A screenshot the assistant asked for, waiting for the window to draw it.
struct Shot {
    call: Call,
    path: PathBuf,
    asked: std::time::Instant,
    /// What the model is told with the picture.
    caption: String,
}

/// A plugin's command sent, and its log watched for how it went.
struct PluginWait {
    call: Call,
    plugin: String,
    command: String,
    /// How many lines the plugin had logged when it was sent.
    before: u64,
    asked: std::time::Instant,
}

/// How long a plugin's command is watched before the model is told it is
/// still going.
const PLUGIN_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Background checks started from the providers window.
enum Check {
    Models(Receiver<Result<Vec<String>, String>>),
    Test(Receiver<Result<String, String>>),
}

/// The provider being added or edited.
struct Draft {
    /// The provider being replaced, if editing.
    original: Option<String>,
    provider: Provider,
    key_mode: KeyMode,
    /// A newly typed key. Empty with [`KeyMode::Stored`] keeps the old one.
    key_input: String,
    env_name: String,
    models: Vec<String>,
    check: Option<Check>,
    status: Option<Result<String, String>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyMode {
    None,
    Stored,
    Env,
}

pub enum Action {
    None,
    Ask(String),
    Stop,
    UndoTurn,
}

#[derive(Default)]
pub struct Assistant {
    pub open: bool,
    settings: Settings,
    settings_path: Option<PathBuf>,
    session: Option<Session>,
    transcript: Vec<Entry>,
    input: String,
    calls: VecDeque<Call>,
    results: Vec<Part>,
    shot: Option<Shot>,
    plugin_wait: Option<PluginWait>,
    /// The undo depth when the current request began, and after the
    /// assistant's last change — for taking back everything one request did.
    turn_start: Option<usize>,
    /// The history as the request left it: its depth, and the bus's change
    /// count, which is what tells "nothing done since" from "undone by hand
    /// and as many edits made again".
    turn_end: Option<(usize, u64)>,
    providers_open: bool,
    draft: Option<Draft>,
    focus_input: bool,
    /// Whether the Claude app lists Tequila, and when that was last looked
    /// at: its settings file is read every few seconds while the window is
    /// open, not every frame.
    claude_app: Option<(std::time::Instant, bool)>,
    /// What connecting the Claude app last said.
    claude_message: Option<Result<String, String>>,
}

impl Assistant {
    /// Load the connected providers from `path`.
    pub fn load(&mut self, path: PathBuf) {
        let (settings, complaints) = Settings::load(&path);
        self.settings = settings;
        self.settings_path = Some(path);
        for complaint in complaints {
            self.transcript.push(Entry::Problem(complaint));
        }
    }

    fn save_settings(&mut self) {
        if let Some(path) = &self.settings_path {
            if let Err(error) = self.settings.save(path) {
                self.transcript.push(Entry::Problem(error));
            }
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.focus_input = self.open;
    }

    fn working(&self) -> bool {
        self.session.as_ref().is_some_and(Session::busy) || !self.calls.is_empty() || self.shot.is_some() || self.plugin_wait.is_some()
    }

    /// Start a request: called by the application, which knows the undo depth.
    pub fn ask(&mut self, text: String, depth: usize) {
        let Some(provider) = self.settings.active().cloned() else {
            self.transcript.push(Entry::Problem("Connect an AI first: Providers…".into()));
            self.providers_open = true;
            return;
        };
        let session = self.session.get_or_insert_with(|| Session::new(provider.clone()));
        // The conversation is kept in a neutral form, so switching provider
        // mid-conversation just sends it to the other one.
        session.provider = provider;
        self.transcript.push(Entry::You(text.clone()));
        match session.ask(&text) {
            Ok(()) => {
                self.turn_start = Some(depth);
                self.turn_end = None;
            }
            Err(error) => self.transcript.push(Entry::Problem(error)),
        }
    }

    pub fn stop(&mut self) {
        if let Some(session) = &mut self.session {
            session.cancel();
        }
        self.calls.clear();
        self.results.clear();
        self.shot = None;
        self.transcript.push(Entry::Note("Stopped.".into()));
    }

    /// Whether "undo what the assistant did" would take back only its work:
    /// something was changed, and nothing has been done by hand since.
    pub fn can_undo_turn(&self, bus: &tequila_scene::CommandBus) -> bool {
        match (self.turn_start, self.turn_end) {
            (Some(start), Some(end)) => end == (bus.depth(), bus.changes()) && bus.depth() > start,
            _ => false,
        }
    }

    /// The request's work no longer applies: another project was opened.
    pub fn forget_turn(&mut self) {
        self.turn_start = None;
        self.turn_end = None;
    }

    pub fn undo_turn(&mut self, app: &mut TequilaApp) {
        let (Some(start), true) = (self.turn_start, self.can_undo_turn(app.project.bus())) else {
            return;
        };
        let camera = app.project.camera().clone();
        while app.project.bus().depth() > start {
            let before = app.project.bus().depth();
            app.project.undo();
            if app.project.bus().depth() >= before {
                break;
            }
        }
        if *app.project.camera() != camera {
            app.camera.adopt(app.project.camera());
        }
        self.turn_end = None;
        self.transcript.push(Entry::Note("Took back everything that request changed.".into()));
    }

    /// Move the conversation along: collect replies, carry out tool calls,
    /// send results. Called every frame.
    pub fn pump(&mut self, app: &mut TequilaApp, ctx: &egui::Context) {
        if let Some(session) = &mut self.session {
            match session.poll() {
                Some(Ok(turn)) => {
                    for text in turn.text {
                        self.transcript.push(Entry::Said(text));
                    }
                    self.calls.extend(turn.calls);
                    if self.calls.is_empty() {
                        self.finish_turn(app);
                    }
                }
                Some(Err(error)) => self.transcript.push(Entry::Problem(error)),
                None => {}
            }
        }

        self.collect_screenshot(ctx);
        self.collect_plugin_run(app);
        while self.shot.is_none() && self.plugin_wait.is_none() {
            let Some(call) = self.calls.pop_front() else { break };
            if !self.carry_out(app, call) {
                // Waiting on something outside the assistant; next frame.
                break;
            }
        }

        let answered = self.calls.is_empty() && self.shot.is_none() && self.plugin_wait.is_none() && !self.results.is_empty();
        if answered {
            let results = std::mem::take(&mut self.results);
            if let Some(session) = &mut self.session {
                if let Err(error) = session.answer(results) {
                    self.transcript.push(Entry::Problem(error));
                }
            }
        }
        if self.working() {
            // Replies arrive on another thread; keep looking for them even
            // when nobody is moving the mouse.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn finish_turn(&mut self, app: &TequilaApp) {
        let depth = app.project.bus().depth();
        if self.turn_start.is_some_and(|start| depth > start) {
            self.turn_end = Some((depth, app.project.bus().changes()));
        }
    }

    /// Carry out one call. False when it has to wait for a later frame.
    fn carry_out(&mut self, app: &mut TequilaApp, call: Call) -> bool {
        let label = step_label(&call);
        let arguments = match call.arguments.clone() {
            Ok(arguments) => arguments,
            Err(error) => {
                self.result(call, error, None, true, label);
                return true;
            }
        };
        match tools::plan(&call.name, &arguments) {
            Err(error) => self.result(call, error, None, true, label),
            Ok(tools::Plan::Screenshot { viewport }) => {
                if app.pending_screenshot.is_some() {
                    // Someone else's capture is in flight; ask again next frame.
                    self.calls.push_front(call);
                    return false;
                }
                let path = std::env::temp_dir().join(format!(
                    "tequila-assistant-{}-{}.png",
                    std::process::id(),
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
                ));
                app.pending_screenshot = Some(path.clone());
                app.screenshot_viewport = viewport;
                self.transcript.push(Entry::Step { text: label, ok: true, detail: String::new() });
                self.shot = Some(Shot { call, path, asked: std::time::Instant::now(), caption: "The screenshot, as the application shows it now.".into() });
            }
            Ok(tools::Plan::Material { json, picture }) => {
                if self.shot.is_some() || app.pending_screenshot.is_some() {
                    // A picture is on its way already; ask again next frame.
                    self.calls.push_front(call);
                    return false;
                }
                let mut request = match parse_request(&json) {
                    Ok(request) => request,
                    Err(error) => {
                        self.result(call, error.to_string(), None, true, label);
                        return true;
                    }
                };
                let name = self.session.as_ref().map(|s| s.provider.name.clone()).unwrap_or_default();
                request.peer = format!("assistant · {name}");
                let path = std::env::temp_dir().join(format!(
                    "tequila-assistant-material-{}-{}.png",
                    std::process::id(),
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
                ));
                let previewing = matches!(&request.operation, tequila_scripting::protocol::Operation::Material(m) if m.action == "preview");
                if let tequila_scripting::protocol::Operation::Material(material) = &mut request.operation {
                    if previewing {
                        material.path = Some(path.display().to_string());
                    }
                }
                let answer = match crate::channel::handle(app, &request) {
                    Reply::Value(value) => tools::describe(&tools::Shape::Raw, &value),
                    Reply::Done => "done".into(),
                    Reply::Failed(message) => {
                        self.result(call, message, None, true, label);
                        return true;
                    }
                };
                // The material made or changed, shown on the ball as well.
                let shown = previewing || (picture && {
                    let made = tequila_format::json::parse(answer.as_bytes()).ok().and_then(|v| v.get("material").and_then(|m| m.as_index()));
                    request.operation = tequila_scripting::protocol::Operation::Material(tequila_scripting::protocol::MaterialRequest {
                        action: "preview".into(),
                        material: made.map(|m| m as u64),
                        path: Some(path.display().to_string()),
                        ..Default::default()
                    });
                    made.is_some() && matches!(crate::channel::handle(app, &request), Reply::Value(_))
                });
                let depth = app.project.bus().depth();
                if self.turn_start.is_some_and(|start| depth > start) {
                    self.turn_end = Some((depth, app.project.bus().changes()));
                }
                self.transcript.push(Entry::Step { text: label, ok: true, detail: String::new() });
                if shown {
                    self.shot = Some(Shot { call, path, asked: std::time::Instant::now(), caption: answer });
                } else {
                    self.results.push(Part::Result { id: call.id, name: call.name, text: answer, image: None, is_error: false });
                }
            }
            Ok(tools::Plan::PluginRun { json, plugin, command }) => {
                let log_total = |app: &mut TequilaApp, plugin: &str| -> u64 {
                    app.plugins.logged.get(plugin).copied().unwrap_or(0)
                };
                let before = log_total(app, &plugin);
                let mut request = match parse_request(&json) {
                    Ok(request) => request,
                    Err(error) => {
                        self.result(call, error.to_string(), None, true, label);
                        return true;
                    }
                };
                let name = self.session.as_ref().map(|s| s.provider.name.clone()).unwrap_or_default();
                request.peer = format!("assistant · {name}");
                match crate::channel::handle(app, &request) {
                    Reply::Failed(message) => self.result(call, message, None, true, label),
                    Reply::Value(value) if value.get("opened_panel").is_some() => {
                        self.result(call, tools::describe(&tools::Shape::Raw, &value), None, false, label)
                    }
                    _ => {
                        self.transcript.push(Entry::Step { text: label, ok: true, detail: String::new() });
                        self.plugin_wait = Some(PluginWait { call, plugin, command, before, asked: std::time::Instant::now() });
                    }
                }
            }
            Ok(tools::Plan::Request { json, shape }) => {
                let mut request = match parse_request(&json) {
                    Ok(request) => request,
                    Err(error) => {
                        self.result(call, error.to_string(), None, true, label);
                        return true;
                    }
                };
                // Named, so the console shows who is acting.
                let name = self.session.as_ref().map(|s| s.provider.name.clone()).unwrap_or_default();
                request.peer = format!("assistant · {name}");
                match crate::channel::handle(app, &request) {
                    Reply::Done => self.result(call, "done".into(), None, false, label),
                    Reply::Value(value) => self.result(call, tools::describe(&shape, &value), None, false, label),
                    Reply::Failed(message) => self.result(call, message, None, true, label),
                }
                let depth = app.project.bus().depth();
                if self.turn_start.is_some_and(|start| depth > start) {
                    self.turn_end = Some((depth, app.project.bus().changes()));
                }
            }
        }
        true
    }

    fn result(&mut self, call: Call, text: String, image: Option<String>, is_error: bool, label: String) {
        let detail = if is_error { text.clone() } else { String::new() };
        if image.is_none() {
            self.transcript.push(Entry::Step { text: label, ok: !is_error, detail });
        }
        self.results.push(Part::Result { id: call.id, name: call.name, text, image, is_error });
    }

    /// Tell the model how a plugin's command went once its log says - or,
    /// after a while, what it has logged so far.
    fn collect_plugin_run(&mut self, app: &mut TequilaApp) {
        let Some(wait) = &self.plugin_wait else { return };
        let total = app.plugins.logged.get(&wait.plugin).copied().unwrap_or(0);
        let lines: Vec<String> = app
            .plugins
            .logs
            .get(&wait.plugin)
            .map(|log| log.iter().skip(log.len().saturating_sub(total.saturating_sub(wait.before) as usize)).cloned().collect())
            .unwrap_or_default();
        let finished = format!("finished `{}`", wait.command);
        let done = lines.contains(&finished);
        let stopped = !app.plugins.hosts.contains_key(&wait.plugin);
        if !done && !stopped && wait.asked.elapsed() < PLUGIN_WAIT {
            return;
        }
        let wait = self.plugin_wait.take().expect("checked above");
        let mut out = tequila_format::json_write::Object::new();
        out.set("finished", done)
            .set("log", tequila_format::json::Value::Array(lines.into_iter().map(tequila_format::json::Value::String).collect()));
        if !done {
            out.set("note", if stopped { "the plugin is not running; tequila_plugin_list says why" } else { "still running; tequila_plugin_log shows more later" });
        }
        let text = tequila_format::json_write::to_string(&out.build());
        self.results.push(Part::Result { id: wait.call.id, name: wait.call.name, text, image: None, is_error: false });
    }

    /// Hand a finished screenshot to the model, and show it here.
    fn collect_screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &self.shot else { return };
        let ready = std::fs::read(&shot.path).ok().filter(|bytes| bytes.ends_with(b"IEND\xaeB`\x82"));
        let Some(bytes) = ready else {
            if shot.asked.elapsed() > std::time::Duration::from_secs(10) {
                let shot = self.shot.take().expect("checked above");
                self.result(shot.call, "the screenshot did not arrive".into(), None, true, "screenshot".into());
            }
            return;
        };
        let shot = self.shot.take().expect("checked above");
        let _ = std::fs::remove_file(&shot.path);
        match prepare_image(&bytes) {
            Some((png, image)) => {
                let texture = ctx.load_texture(format!("assistant-shot-{}", self.transcript.len()), image, egui::TextureOptions::LINEAR);
                self.transcript.push(Entry::Picture(texture));
                self.results.push(Part::Result { id: shot.call.id, name: shot.call.name, text: shot.caption, image: Some(png), is_error: false });
            }
            None => self.result(shot.call, "the screenshot could not be read".into(), None, true, "screenshot".into()),
        }
    }

    // ---- the panel ---------------------------------------------------------------

    pub fn show(&mut self, ui: &mut egui::Ui, can_undo_turn: bool) -> Action {
        let mut action = Action::None;

        egui::TopBottomPanel::top("assistant-header").show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Assistant").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Providers…").on_hover_text("Connect Claude, OpenAI, a local model or your company's AI").clicked() {
                        self.providers_open = true;
                    }
                    if ui.small_button("New chat").on_hover_text("Start a fresh conversation").clicked() && !self.working() {
                        self.session = None;
                        self.transcript.clear();
                        self.turn_start = None;
                        self.turn_end = None;
                    }
                });
            });
            if !self.settings.providers.is_empty() {
                let current = self.settings.active().map(|p| format!("{} · {}", p.name, p.model)).unwrap_or_else(|| "Choose an AI".into());
                egui::ComboBox::from_id_salt("assistant-provider")
                    .selected_text(current)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        let mut chosen = None;
                        for provider in &self.settings.providers {
                            let active = self.settings.active.as_deref() == Some(provider.id.as_str());
                            if ui.selectable_label(active, format!("{} · {}", provider.name, provider.model)).clicked() {
                                chosen = Some(provider.id.clone());
                            }
                        }
                        if let Some(id) = chosen {
                            self.settings.active = Some(id);
                            self.save_settings();
                        }
                    });
            }
            ui.add_space(4.0);
        });

        egui::TopBottomPanel::bottom("assistant-prompt").show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if self.working() {
                    ui.spinner();
                    let step = self.session.as_ref().map(Session::steps).unwrap_or(0);
                    let what = if self.shot.is_some() { "Looking" } else if self.calls.is_empty() { "Thinking" } else { "Working" };
                    ui.label(egui::RichText::new(if step > 0 { format!("{what} · step {}", step + 1) } else { format!("{what}…") }).color(crate::theme::dim()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(can_undo_turn && !self.working(), egui::Button::new("Undo request").small())
                        .on_hover_text("Take back everything the last request changed")
                        .clicked()
                    {
                        action = Action::UndoTurn;
                    }
                });
            });
            let id = ui.make_persistent_id("assistant-input");
            // Enter sends; Shift+Enter is a new line.
            let enter = ui.memory(|m| m.has_focus(id))
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.input)
                    .id(id)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("Ask for anything — \"make the strap black velvet and light it like a studio shot\""),
            );
            if std::mem::take(&mut self.focus_input) {
                response.request_focus();
            }
            ui.horizontal(|ui| {
                if self.working() {
                    if ui.button("Stop").clicked() {
                        action = Action::Stop;
                    }
                } else {
                    let send = ui.add_enabled(!self.input.trim().is_empty(), egui::Button::new(egui::RichText::new("Send").strong()));
                    if (send.clicked() || enter) && !self.input.trim().is_empty() {
                        action = Action::Ask(std::mem::take(&mut self.input).trim().to_string());
                    }
                }
                ui.label(egui::RichText::new("Enter to send · Shift+Enter for a new line").small().color(crate::theme::dim()));
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                if self.settings.providers.is_empty() && self.transcript.is_empty() {
                    ui.add_space(12.0);
                    ui.label("Describe what you want, and an AI sets it up in the scene: materials, textures, lighting, camera.");
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("It works with Claude, OpenAI, a model running on this computer (Ollama), or your company's own AI.").color(crate::theme::dim()));
                    ui.add_space(8.0);
                    if ui.button("Connect an AI…").clicked() {
                        self.providers_open = true;
                    }
                    return;
                }
                for entry in &self.transcript {
                    match entry {
                        Entry::You(text) => {
                            ui.add_space(6.0);
                            egui::Frame::none().fill(crate::theme::p().card).rounding(8.0).inner_margin(8.0).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(text);
                            });
                        }
                        Entry::Said(text) => {
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(text.as_str()).wrap());
                        }
                        Entry::Step { text, ok, detail } => {
                            let colour = if *ok { crate::theme::dim() } else { crate::theme::p().danger };
                            let mark = if *ok { "·" } else { "×" };
                            let label = ui.label(egui::RichText::new(format!("{mark} {text}")).small().monospace().color(colour));
                            if !detail.is_empty() {
                                label.on_hover_text(detail);
                            }
                        }
                        Entry::Picture(texture) => {
                            let size = texture.size_vec2();
                            let width = ui.available_width().min(size.x).min(320.0);
                            ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(width, width * size.y / size.x.max(1.0))).rounding(4.0));
                        }
                        Entry::Problem(text) => {
                            ui.add(egui::Label::new(egui::RichText::new(text).color(crate::theme::p().danger)).wrap());
                        }
                        Entry::Note(text) => {
                            ui.label(egui::RichText::new(text).small().italics().color(crate::theme::dim()));
                        }
                    }
                }
            });
        });
        action
    }

    // ---- connecting providers -------------------------------------------------------

    /// Whether the Connect an AI window is open.
    pub fn providers_open(&self) -> bool {
        self.providers_open
    }

    pub fn providers_window(&mut self, ctx: &egui::Context) {
        if !self.providers_open {
            return;
        }
        let mut open = true;
        let spec = crate::windows::Spec {
            kind: crate::windows::Kind::Connect,
            title: "Connect an AI".into(),
            size: egui::vec2(560.0, 480.0),
            min_size: egui::vec2(420.0, 300.0),
            resizable: true,
            margin: 16.0,
        };
        crate::windows::show(ctx, spec, &mut open, |window| window.default_width(480.0).resizable(true), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                if self.draft.is_some() {
                    self.edit_draft(ui);
                    return;
                }
                self.claude_app_section(ui);
                ui.separator();
                ui.label(egui::RichText::new("Tequila's Assistant").strong());
                if self.settings.providers.is_empty() {
                    ui.label("Nothing connected yet.");
                }
                let mut remove = None;
                let mut edit = None;
                for provider in &self.settings.providers {
                    ui.horizontal(|ui| {
                        let active = self.settings.active.as_deref() == Some(provider.id.as_str());
                        if ui.radio(active, "").on_hover_text("Use this one").clicked() {
                            self.settings.active = Some(provider.id.clone());
                        }
                        ui.label(egui::RichText::new(&provider.name).strong());
                        ui.label(egui::RichText::new(format!("{} · {}", provider.kind.label(), provider.model)).color(crate::theme::dim()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Remove").clicked() {
                                remove = Some(provider.id.clone());
                            }
                            if ui.small_button("Edit").clicked() {
                                edit = Some(provider.clone());
                            }
                        });
                    });
                }
                if let Some(id) = remove {
                    self.settings.providers.retain(|p| p.id != id);
                    if self.settings.active.as_deref() == Some(id.as_str()) {
                        self.settings.active = self.settings.providers.first().map(|p| p.id.clone());
                    }
                    self.save_settings();
                }
                if let Some(provider) = edit {
                    self.draft = Some(Draft::of(Some(provider.id.clone()), provider));
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.label("Add:");
                    for preset in Provider::presets() {
                        if ui.button(&preset.name).clicked() {
                            let mut provider = preset;
                            provider.id = self.settings.new_id(&provider.name);
                            self.draft = Some(Draft::of(None, provider));
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(if tequila_ai::secret::encrypted() {
                        "API keys are encrypted for your Windows account, and are sent only to the service they belong to."
                    } else {
                        "API keys are stored encoded but not encrypted on this system; an environment variable is safer."
                    })
                    .small()
                    .color(crate::theme::dim()),
                );
            });
        });
        if !open {
            self.providers_open = false;
            self.draft = None;
        }
    }

    /// The Claude desktop app, connected to Tequila's tools in one click,
    /// so asked to work in Tequila it uses them rather than the mouse.
    fn claude_app_section(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("The Claude app").strong());
        ui.label(
            egui::RichText::new(
                "Lets the Claude desktop app work in Tequila through its tools - exact, fast and undoable - \
                 instead of clicking around the screen.",
            )
            .small()
            .color(crate::theme::dim()),
        );
        let Some(program) = crate::connect_claude::server_program() else {
            ui.label(
                egui::RichText::new("Tequila's command-line program is not beside the application, so the Claude app cannot be connected from here.")
                    .small()
                    .color(crate::theme::dim()),
            );
            return;
        };
        let paths = crate::connect_claude::config_paths();
        let stale = self.claude_app.is_none_or(|(at, _)| at.elapsed() > std::time::Duration::from_secs(3));
        if stale {
            let connected = paths.iter().any(|path| crate::connect_claude::is_connected(path, &program));
            self.claude_app = Some((std::time::Instant::now(), connected));
        }
        let connected = self.claude_app.is_some_and(|(_, c)| c);
        ui.horizontal_wrapped(|ui| {
            if connected {
                ui.colored_label(crate::theme::p().success, "Connected");
            }
            if ui.button(if connected { "Connect again" } else { "Connect the Claude app" }).clicked() {
                let failures: Vec<String> = paths.iter().filter_map(|path| crate::connect_claude::connect(path, &program).err()).collect();
                self.claude_message = Some(if failures.is_empty() {
                    Ok("Done. Quit the Claude app completely (right-click its icon by the clock, Quit) and open it again; \
                        then ask it to do something in Tequila."
                        .into())
                } else {
                    Err(failures.join("\n"))
                });
                self.claude_app = None;
            }
            if ui
                .small_button("Copy for Claude Code")
                .on_hover_text(crate::connect_claude::claude_code_command(&program))
                .clicked()
            {
                ui.ctx().copy_text(crate::connect_claude::claude_code_command(&program));
                self.claude_message = Some(Ok("Copied. Paste it into a terminal where Claude Code runs.".into()));
            }
        });
        // The skill that teaches Claude to work in Tequila: what to look at
        // first, the library, colours, floor reflections, renders.
        if let Some(skill) = crate::connect_claude::skill_file() {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Better results: add Tequila's skill to Claude: in its Settings, Capabilities, Skills, upload the file.")
                        .small()
                        .color(crate::theme::dim()),
                );
                if ui.small_button("Show the skill file").clicked() {
                    crate::library_tab::reveal(&skill.to_string_lossy());
                }
                if let Some(skills) = crate::connect_claude::claude_code_skills() {
                    if ui.small_button("Add it to Claude Code").on_hover_text(skills.join("tequila").display().to_string()).clicked() {
                        self.claude_message = Some(crate::connect_claude::install_skill(&skill, &skills).map(|folder| {
                            format!("Added to Claude Code's skills ({}): new Claude Code sessions use it.", folder.display())
                        }));
                    }
                }
            });
        }
        match &self.claude_message {
            Some(Ok(text)) => {
                ui.label(egui::RichText::new(text).small().color(crate::theme::p().success));
            }
            Some(Err(text)) => {
                ui.add(egui::Label::new(egui::RichText::new(text).small().color(crate::theme::p().danger)).wrap());
            }
            None => {}
        }
    }

    fn edit_draft(&mut self, ui: &mut egui::Ui) {
        let Some(draft) = &mut self.draft else { return };
        draft.poll();
        let provider = &mut draft.provider;
        egui::Grid::new("provider-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut provider.name);
            ui.end_row();

            ui.label("Type");
            egui::ComboBox::from_id_salt("provider-kind").selected_text(provider.kind.label()).show_ui(ui, |ui| {
                for kind in Kind::ALL {
                    if ui.selectable_label(provider.kind == kind, kind.label()).clicked() && provider.kind != kind {
                        if provider.base_url.is_empty() || provider.base_url == provider.kind.default_url() {
                            provider.base_url = kind.default_url().into();
                        }
                        provider.kind = kind;
                    }
                }
            });
            ui.end_row();

            ui.label("Address");
            ui.add(egui::TextEdit::singleline(&mut provider.base_url).desired_width(320.0).hint_text(match provider.kind {
                Kind::Anthropic => "https://api.anthropic.com",
                Kind::OpenAi => "https://…/v1",
            }));
            ui.end_row();

            ui.label("Model");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut provider.model).desired_width(220.0).hint_text("the model's id"));
                if !draft.models.is_empty() {
                    egui::ComboBox::from_id_salt("provider-models").selected_text("Choose…").show_ui(ui, |ui| {
                        for model in &draft.models {
                            if ui.selectable_label(provider.model == *model, model).clicked() {
                                provider.model = model.clone();
                            }
                        }
                    });
                }
            });
            ui.end_row();

            ui.label("Key");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut draft.key_mode, KeyMode::Stored, "API key");
                    ui.radio_value(&mut draft.key_mode, KeyMode::Env, "Environment variable");
                    ui.radio_value(&mut draft.key_mode, KeyMode::None, "None (local server)");
                });
                match draft.key_mode {
                    KeyMode::Stored => {
                        let kept = matches!(provider.key, Key::Stored(_));
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.key_input)
                                .password(true)
                                .desired_width(320.0)
                                .hint_text(if kept { "saved — type a new one to replace it" } else { "paste the API key" }),
                        );
                    }
                    KeyMode::Env => {
                        ui.add(egui::TextEdit::singleline(&mut draft.env_name).desired_width(220.0).hint_text("e.g. ANTHROPIC_API_KEY"));
                    }
                    KeyMode::None => {}
                }
                // Someone given a key and nothing else: what the key says
                // about where it belongs, filled in where that is certain.
                if draft.key_mode == KeyMode::Stored && !draft.key_input.trim().is_empty() {
                    match tequila_ai::provider::recognise_key(&draft.key_input) {
                        tequila_ai::provider::KeyLooks::Known { service, kind, base_url } => {
                            if provider.base_url.trim().is_empty() || provider.kind != kind && provider.base_url == provider.kind.default_url() {
                                provider.kind = kind;
                                provider.base_url = base_url.into();
                            }
                            ui.label(
                                egui::RichText::new(format!("A {service} key: its address is filled in. Click Fetch models to choose one."))
                                    .small()
                                    .color(crate::theme::p().success),
                            );
                        }
                        tequila_ai::provider::KeyLooks::OpenAiOrGateway if provider.base_url.trim().is_empty() => {
                            ui.label(
                                egui::RichText::new(
                                    "This could be an OpenAI key or one from a company gateway, which use the same start. \
                                     If it came from Uliverse or your IT team, ask them for the API address too.",
                                )
                                .small()
                                .color(crate::theme::dim()),
                            );
                            if ui.small_button("It's an OpenAI key").clicked() {
                                provider.kind = Kind::OpenAi;
                                provider.base_url = Kind::OpenAi.default_url().into();
                            }
                        }
                        tequila_ai::provider::KeyLooks::Unknown if provider.base_url.trim().is_empty() => {
                            ui.label(
                                egui::RichText::new(
                                    "This key does not say which service it belongs to, so Tequila needs the service's \
                                     address as well. Ask whoever gave you the key for \"the API base URL\" (it often ends \
                                     in /v1) and whether it is OpenAI- or Anthropic-compatible.",
                                )
                                .small()
                                .color(crate::theme::dim()),
                            );
                        }
                        _ => {}
                    }
                }
            });
            ui.end_row();
        });

        crate::field::Fold::new("Advanced").id_salt("provider-advanced").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Key header");
                ui.add(egui::TextEdit::singleline(&mut provider.key_header).desired_width(160.0).hint_text(match provider.kind {
                    Kind::Anthropic => "x-api-key",
                    Kind::OpenAi => "Authorization: Bearer",
                }))
                .on_hover_text("For gateways that expect the key in another header, such as Azure's api-key");
            });
            ui.add(crate::field::Checkbox::new(&mut provider.vision, "The model can see images (it checks its work with screenshots)"));
            ui.horizontal(|ui| {
                ui.label("Longest reply");
                ui.add(egui::DragValue::new(&mut provider.max_tokens).range(256..=64_000).suffix(" tokens"));
            });
        });

        if provider.id == "custom" || provider.id.starts_with("company") || provider.name.to_lowercase().contains("uliverse") {
            ui.label(
                egui::RichText::new(
                    "For a company AI such as Uliverse, ask your IT or AI team for: an OpenAI-compatible (or Anthropic-compatible) \
                     API address, a model name, and an API key — and, if they use one, the name of the header the key goes in.",
                )
                .small()
                .color(crate::theme::dim()),
            );
        }

        ui.add_space(6.0);
        let busy = draft.check.is_some();
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new("Fetch models")).on_hover_text("Ask the service which models it has").clicked() {
                let candidate = draft.candidate();
                let (sender, receiver) = channel();
                std::thread::spawn(move || {
                    let _ = sender.send(candidate.and_then(|p| session::fetch_models(&p)));
                });
                draft.check = Some(Check::Models(receiver));
                draft.status = None;
            }
            if ui.add_enabled(!busy, egui::Button::new("Test")).on_hover_text("Send a one-line message to check it answers").clicked() {
                let candidate = draft.candidate();
                let (sender, receiver) = channel();
                std::thread::spawn(move || {
                    let _ = sender.send(candidate.and_then(|p| session::test(&p)));
                });
                draft.check = Some(Check::Test(receiver));
                draft.status = None;
            }
            if busy {
                ui.spinner();
            }
        });
        match &draft.status {
            Some(Ok(text)) => {
                ui.colored_label(crate::theme::p().success, text);
            }
            Some(Err(text)) => {
                ui.add(egui::Label::new(egui::RichText::new(text).color(crate::theme::p().danger)).wrap());
            }
            None => {}
        }

        ui.separator();
        let (mut save, mut cancel) = (false, false);
        ui.horizontal(|ui| {
            save = ui.button(egui::RichText::new("Save").strong()).clicked();
            cancel = ui.button("Cancel").clicked();
        });
        let mut done = cancel;
        if save {
            match draft.candidate() {
                Ok(provider) => match provider.problem() {
                    Some(problem) => draft.status = Some(Err(format!("Not saved: {problem}"))),
                    None => {
                        let id = provider.id.clone();
                        match draft.original.clone().and_then(|o| self.settings.providers.iter().position(|p| p.id == o)) {
                            Some(index) => self.settings.providers[index] = provider,
                            None => self.settings.providers.push(provider),
                        }
                        if self.settings.active().is_none() {
                            self.settings.active = Some(id);
                        }
                        done = true;
                    }
                },
                Err(error) => draft.status = Some(Err(error)),
            }
        }
        if done {
            self.draft = None;
            if save {
                self.save_settings();
            }
        }
    }
}

impl Draft {
    fn of(original: Option<String>, provider: Provider) -> Draft {
        let (key_mode, env_name) = match &provider.key {
            Key::None if provider.kind == Kind::OpenAi && provider.base_url.contains("localhost") => (KeyMode::None, String::new()),
            Key::None => (KeyMode::Stored, String::new()),
            Key::Stored(_) => (KeyMode::Stored, String::new()),
            Key::Env(name) => (KeyMode::Env, name.clone()),
        };
        Draft { original, provider, key_mode, key_input: String::new(), env_name, models: Vec::new(), check: None, status: None }
    }

    /// The provider as the form stands, key included.
    fn candidate(&self) -> Result<Provider, String> {
        let mut provider = self.provider.clone();
        provider.name = provider.name.trim().to_string();
        provider.base_url = provider.base_url.trim().to_string();
        provider.model = provider.model.trim().to_string();
        provider.key = match self.key_mode {
            KeyMode::None => Key::None,
            KeyMode::Env => {
                let name = self.env_name.trim();
                if name.is_empty() {
                    return Err("name the environment variable that holds the key".into());
                }
                Key::Env(name.to_string())
            }
            KeyMode::Stored => {
                let typed = self.key_input.trim();
                match (&self.provider.key, typed.is_empty()) {
                    (_, false) => Key::Stored(typed.to_string()),
                    (Key::Stored(old), true) => Key::Stored(old.clone()),
                    (_, true) => return Err("paste the API key".into()),
                }
            }
        };
        Ok(provider)
    }

    fn poll(&mut self) {
        let finished = match &self.check {
            Some(Check::Models(receiver)) => receiver.try_recv().ok().map(|result| match result {
                Ok(models) if models.is_empty() => Err("the service listed no models; type the model's id".to_string()),
                Ok(models) => {
                    let count = models.len();
                    self.models = models;
                    Ok(format!("{count} models found — choose one"))
                }
                Err(error) => Err(error),
            }),
            Some(Check::Test(receiver)) => receiver.try_recv().ok().map(|result| result.map(|reply| format!("Connected — it answered: {reply}"))),
            None => None,
        };
        if let Some(status) = finished {
            self.status = Some(status);
            self.check = None;
        }
    }
}

/// A short line for the panel: what the assistant is doing.
fn step_label(call: &Call) -> String {
    let name = call.name.trim_start_matches("tequila_");
    let arguments = call.arguments.as_ref().ok();
    let detail = match name {
        "apply" => arguments.and_then(|a| a.get("command")).and_then(|c| c.get("command")).and_then(|c| c.as_str()).map(str::to_string),
        "batch" => arguments.and_then(|a| a.get("ops")).map(|ops| format!("{} changes as one step", ops.as_array().len())),
        "nodes" => arguments.and_then(|a| a.get("filter")).and_then(|f| f.as_str()).map(|f| format!("\"{f}\"")),
        "look" => arguments.and_then(|a| a.get("view")).and_then(|v| v.as_str()).map(str::to_string),
        "ui" => arguments.and_then(|a| a.get("viewport")).and_then(|v| v.as_str()).map(|v| format!("viewport {v}")),
        _ => None,
    };
    match detail {
        Some(detail) => format!("{name} {detail}"),
        None => name.to_string(),
    }
}

/// A PNG screenshot shrunk for sending, as base64 for the model and as an
/// image for the panel.
fn prepare_image(png_bytes: &[u8]) -> Option<(String, egui::ColorImage)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    let (width, height, rgba) = crate::render_window::shrink(info.width as usize, info.height as usize, &buffer[..info.buffer_size()], SENT_SIZE);
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
    Some((tequila_ai::base64::encode(&encoded), image))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_format::json;

    fn call(name: &str, arguments: &str) -> Call {
        Call { id: "c".into(), name: name.into(), arguments: Ok(json::parse(arguments.as_bytes()).unwrap()) }
    }

    /// The assistant's tool calls run through the real handler on a real
    /// application: they change the scene, they are undoable, and a failing
    /// one comes back as an error the model can read.
    #[test]
    fn tool_calls_change_the_scene_through_the_handler() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        let node = app.project.document().tree.iter().next().unwrap().id.0;
        let start = app.project.bus().depth();
        assistant.turn_start = Some(start);

        assistant.carry_out(&mut app, call("tequila_apply", &format!(r#"{{"command":{{"command":"scene.rename","node":{node},"name":"By the assistant"}}}}"#)));
        assistant.carry_out(&mut app, call("tequila_select", r#"{"nodes":[99999999]}"#));

        assert_eq!(app.project.document().tree.get(tequila_scene::tree::NodeId(node)).unwrap().name, "By the assistant");
        let errors: Vec<bool> = assistant.results.iter().map(|r| matches!(r, Part::Result { is_error: true, .. })).collect();
        assert_eq!(errors, [false, true]);
        assert!(app.console.text().contains("assistant"), "the console shows who acted");

        // Undo request takes back exactly what it did.
        assert!(assistant.can_undo_turn(app.project.bus()));
        assistant.undo_turn(&mut app);
        assert_eq!(app.project.bus().depth(), start);
        assert_ne!(app.project.document().tree.get(tequila_scene::tree::NodeId(node)).unwrap().name, "By the assistant");
    }

    /// A material the assistant makes goes on the part, and comes back to it
    /// with a picture of it on the ball, so it can see what it made.
    #[test]
    fn a_material_made_by_the_assistant_comes_with_its_picture() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        let part = app.project.document().tree.iter().find(|n| n.mesh().is_some()).unwrap().id;
        assistant.turn_start = Some(app.project.bus().depth());
        let recipe = r##"{"name": "Test Speckle", "type": "plastic", "patterns": [{"param": "base_color", "procedural": {"kind": "spots"}, "size": 0.004, "colors": [{"at": 0, "color": "#eeeeee"}, {"at": 1, "color": "#111111"}]}]}"##;
        assert!(assistant.carry_out(&mut app, call("tequila_material_create", &format!(r#"{{"recipe":{recipe},"nodes":[{}]}}"#, part.0))));
        let key = app.project.document().tree.get(part).unwrap().material.expect("a material on the part");
        assert_eq!(app.project.document().materials.get(key).unwrap().name, "Test Speckle");
        let shot = assistant.shot.as_ref().expect("its picture is on its way");
        assert!(shot.path.exists(), "the preview was not written");
        assert!(shot.caption.contains("Test Speckle"), "{}", shot.caption);
        let _ = std::fs::remove_file(&shot.path);
        // One step to take back.
        assert!(assistant.can_undo_turn(app.project.bus()));
    }

    /// A plugin's command run by the assistant is answered once it has run,
    /// with what it printed and that it finished - the frames go on, the
    /// window is not held. Skipped where there is no Python.
    #[test]
    fn a_plugin_run_by_the_assistant_answers_with_its_log() {
        if crate::plugins::python().is_none() {
            eprintln!("skipped: no Python 3");
            return;
        }
        let root = std::env::temp_dir().join(format!("tequila-assistant-plugin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join("test.echo");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("plugin.json"), r#"{"id":"test.echo","name":"Echo","main":"main.py","commands":[{"id":"go"}]}"#).unwrap();
        std::fs::write(folder.join("main.py"), "import tequila_plugin as plugin\n\n@plugin.command('go')\ndef go(fs, values):\n    print('hello ' + str(values.get('n')))\n").unwrap();
        let mut app = TequilaApp::for_tests();
        app.plugins = crate::plugins::Plugins::load_from(vec![(root.clone(), crate::plugins::Origin::User)], root.join("enabled.json"));
        app.plugins.set_enabled("test.echo", true).unwrap();
        app.plugins.start_enabled(None);
        // Up first: a first start of Python can take longer than the run is
        // watched for.
        let ready = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !app.plugins.logs.get("test.echo").is_some_and(|l| l.contains(&"running".to_string())) && std::time::Instant::now() < ready {
            app.plugins.poll();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let mut assistant = Assistant::default();
        assert!(assistant.carry_out(&mut app, call("tequila_plugin_run", r#"{"id":"test.echo","command":"go","values":{"n":3}}"#)));
        assert!(assistant.plugin_wait.is_some(), "the run was answered before it ran");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while assistant.plugin_wait.is_some() && std::time::Instant::now() < deadline {
            app.plugins.poll();
            assistant.collect_plugin_run(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let Some(Part::Result { text, is_error, .. }) = assistant.results.last() else { panic!("no answer") };
        let answer = tequila_format::json::parse(text.as_bytes()).expect("the answer is JSON");
        let finished = answer.get("finished").and_then(|v| v.as_bool());
        let said: Vec<&str> = answer.get("log").map(|l| l.as_array().iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
        assert!(!is_error && finished == Some(true) && said.contains(&"hello 3"), "{text}");
        app.plugins.set_enabled("test.echo", false).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Once the person has edited by hand, undoing "the request" would take
    /// their work too, so it is not offered.
    #[test]
    fn a_manual_edit_after_the_request_blocks_undo_request() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        let node = app.project.document().tree.iter().next().unwrap().id;
        assistant.turn_start = Some(app.project.bus().depth());
        assistant.carry_out(&mut app, call("tequila_apply", &format!(r#"{{"command":{{"command":"scene.rename","node":{},"name":"A"}}}}"#, node.0)));
        app.project.apply(tequila_scene::Command::Rename { node, name: "By hand".into() }).unwrap();
        assert!(!assistant.can_undo_turn(app.project.bus()));
    }

    /// Undone by hand and as many edits made again leaves the depth where
    /// the request left it; the edits are still the person's, not its.
    #[test]
    fn edits_made_after_undoing_the_request_by_hand_are_not_taken_back() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        let node = app.project.document().tree.iter().next().unwrap().id;
        assistant.turn_start = Some(app.project.bus().depth());
        assistant.carry_out(&mut app, call("tequila_apply", &format!(r#"{{"command":{{"command":"scene.rename","node":{},"name":"A"}}}}"#, node.0)));
        assert!(assistant.can_undo_turn(app.project.bus()));
        app.project.undo();
        app.project.apply(tequila_scene::Command::Rename { node, name: "By hand".into() }).unwrap();
        assert!(!assistant.can_undo_turn(app.project.bus()), "the button would undo the person's own edit");
    }

    /// A script's screenshot already in flight makes the assistant's wait a
    /// frame — and the frame must end, not spin on the same call forever.
    #[test]
    fn a_screenshot_waits_for_one_already_in_flight() {
        let mut app = TequilaApp::for_tests();
        let mut assistant = Assistant::default();
        app.pending_screenshot = Some(std::env::temp_dir().join("someone-else.png"));
        assistant.calls.push_back(call("tequila_screenshot", "{}"));
        assistant.pump(&mut app, &egui::Context::default());
        assert_eq!(assistant.calls.len(), 1, "still waiting its turn");
        assert!(assistant.shot.is_none());

        app.pending_screenshot = None;
        assistant.pump(&mut app, &egui::Context::default());
        assert!(assistant.shot.is_some(), "taken once the other capture is done");
        assert!(app.pending_screenshot.is_some());
    }

    #[test]
    fn a_screenshot_is_shrunk_and_encoded_for_the_model() {
        let (w, h) = (2048u32, 1024u32);
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&vec![128; (w * h * 4) as usize]).unwrap();
        }
        let (encoded, image) = prepare_image(&png_bytes).expect("prepared");
        assert!(image.size[0] <= SENT_SIZE && image.size[1] <= SENT_SIZE, "{:?}", image.size);
        let decoded = tequila_ai::base64::decode(&encoded).unwrap();
        assert!(decoded.starts_with(b"\x89PNG"));
    }

    #[test]
    fn steps_are_labelled_for_people() {
        assert_eq!(step_label(&call("tequila_apply", r#"{"command":{"command":"material.set_type"}}"#)), "apply material.set_type");
        assert_eq!(step_label(&call("tequila_batch", r#"{"ops":[{},{},{}]}"#)), "batch 3 changes as one step");
        assert_eq!(step_label(&call("tequila_look", r#"{"view":"front"}"#)), "look front");
    }
}
