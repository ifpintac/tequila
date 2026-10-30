//! The console: a terminal docked under the viewport.
//!
//! One place to type at the application and to watch what else is typing at
//! it. What a person enters is read by [`tequila_scripting::shell`] into the
//! same request a client would send, and answered by the same
//! [`crate::channel::handle`] — so the console is a way to *use* the scripting
//! API, not a second one beside it.
//!
//! Requests that arrive over the socket are written into the same scrollback,
//! in their own colour and with where they came from. That is the other half
//! of the job: when an assistant is driving the scene, the person in front of
//! it reads along in the place they would look anyway, rather than in a log
//! they have to go and open.

use crate::TequilaApp;
use tequila_scripting::protocol::{parse_request, Reply};
use tequila_scripting::shell::{self, Local, Parsed};
use std::collections::VecDeque;

/// Lines kept in the scrollback.
///
/// Rows are drawn only when on screen, so the cost of a long scrollback is
/// memory, not frame time; this bounds that. A `context` query on a split
/// model is a few thousand lines on its own, so it has to be generous.
const KEPT: usize = 20_000;

/// How deep `run` may call `run`. Deep enough for a script that runs a setup
/// script, shallow enough that a script that runs itself stops at once.
const NESTING: usize = 4;

/// The console's colours, from the theme: a terminal in either mode.
pub fn background() -> egui::Color32 {
    crate::theme::p().console
}
fn header() -> egui::Color32 {
    crate::theme::p().chrome
}
fn input_colour() -> egui::Color32 {
    if crate::theme::p().dark { egui::Color32::from_rgb(97, 175, 239) } else { egui::Color32::from_rgb(22, 104, 196) }
}
fn output() -> egui::Color32 {
    crate::theme::p().console_text
}
fn error() -> egui::Color32 {
    crate::theme::p().danger
}
fn info() -> egui::Color32 {
    crate::theme::p().text_faint
}
fn remote() -> egui::Color32 {
    crate::theme::p().busy
}
fn ok() -> egui::Color32 {
    crate::theme::p().success
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Input,
    Output,
    Error,
    Info,
    Success,
    Remote,
}

impl Kind {
    fn colour(self) -> egui::Color32 {
        match self {
            Kind::Input => input_colour(),
            Kind::Output => output(),
            Kind::Error => error(),
            Kind::Info => info(),
            Kind::Success => ok(),
            Kind::Remote => remote(),
        }
    }
}

struct Line {
    kind: Kind,
    text: String,
}

/// Up and Down through what has been typed, as a shell does it.
///
/// Kept apart from the widget so it can be tested: the rule that matters is
/// that walking back through history and then forward past the newest entry
/// gives back the half-typed line you started from, not an empty prompt.
#[derive(Default)]
struct History {
    entries: Vec<String>,
    /// Where Up and Down have got to, while browsing.
    cursor: Option<usize>,
    /// The line as it was before browsing began.
    draft: String,
}

impl History {
    fn push(&mut self, line: &str) {
        self.cursor = None;
        if self.entries.last().map(String::as_str) != Some(line) {
            self.entries.push(line.to_string());
        }
    }

    fn up(&mut self, input: &mut String) {
        let Some(last) = self.entries.len().checked_sub(1) else { return };
        let next = match self.cursor {
            None => {
                self.draft = input.clone();
                last
            }
            Some(i) => i.saturating_sub(1),
        };
        self.cursor = Some(next);
        *input = self.entries[next].clone();
    }

    fn down(&mut self, input: &mut String) {
        match self.cursor {
            None => {}
            Some(i) if i + 1 < self.entries.len() => {
                self.cursor = Some(i + 1);
                *input = self.entries[i + 1].clone();
            }
            Some(_) => {
                self.cursor = None;
                *input = std::mem::take(&mut self.draft);
            }
        }
    }
}

pub struct Console {
    pub open: bool,
    lines: VecDeque<Line>,
    input: String,
    history: History,
    /// The text of the last answer, for `copy`.
    last_answer: String,
    /// Set by `copy`, which runs without a `Ui` to copy with.
    to_clipboard: Option<String>,
    focus: bool,
    /// Put the text cursor at the end of the line on the next frame, after
    /// the line was replaced from outside the widget (history, completion).
    cursor_to_end: bool,
}

impl Default for Console {
    fn default() -> Self {
        let mut console = Console {
            open: false,
            lines: VecDeque::new(),
            input: String::new(),
            history: History::default(),
            last_answer: String::new(),
            to_clipboard: None,
            focus: false,
            cursor_to_end: false,
        };
        console.push(Kind::Info, "Tequila console. Type help for the commands, Tab to complete, Up for history.");
        console.push(
            Kind::Info,
            "Scripts and AI assistants connected to the scripting channel appear here, in amber, as they act.",
        );
        console
    }
}

/// What the panel wants done once it has stopped borrowing the application.
pub enum Action {
    None,
    Run(String),
    /// Pick a file and run it.
    RunFile,
}

/// What the header needs to know that lives elsewhere.
pub struct Status {
    pub listening: Option<u16>,
    pub requests: u64,
    pub active: bool,
}

impl Console {
    fn push(&mut self, kind: Kind, text: &str) {
        for line in text.split('\n') {
            if self.lines.len() == KEPT {
                self.lines.pop_front();
            }
            self.lines.push_back(Line {
                kind,
                text: line.trim_end_matches('\r').to_string(),
            });
        }
    }

    /// Everything in the scrollback, for tests that check what a person
    /// watching would have seen.
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.focus = self.open;
    }

    pub fn show_and_focus(&mut self) {
        self.open = true;
        self.focus = true;
    }

    /// Notes on something done outside the console, written where they
    /// can be read in full: an import's warnings, which the status line has
    /// room for only one of.
    pub fn notes(&mut self, summary: &str, notes: &[String]) {
        self.push(Kind::Info, summary);
        for note in notes {
            self.push(Kind::Info, &format!("    · {note}"));
        }
    }

    /// A request that came in over the socket, written where it can be seen.
    ///
    /// `steps` are the parts of a batch, one line each, so thirty changes to
    /// someone's scene read as thirty changes and not as "batch of 30".
    pub fn remote(&mut self, at: &str, origin: &str, summary: &str, steps: &[String], failure: Option<&str>) {
        self.push(Kind::Remote, &format!("{at}  {origin}  {summary}"));
        for step in steps {
            self.push(Kind::Remote, &format!("            · {step}"));
        }
        if let Some(message) = failure {
            self.push(Kind::Error, &format!("    failed: {message}"));
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, status: &Status) -> Action {
        let mut action = Action::None;
        if let Some(text) = self.to_clipboard.take() {
            ui.output_mut(|o| o.copied_text = text);
        }

        // Three panels inside the console's own, rather than three frames
        // stacked in it. A stacked layout has to predict its own height to
        // size the scrollback, and a prediction a pixel over makes the outer
        // panel grow by that pixel every frame until it has eaten the
        // viewport. Panels are given the space instead of asking for it.

        // --- header -------------------------------------------------------
        egui::TopBottomPanel::top("console-header")
            .frame(
                egui::Frame::none()
                    .fill(header())
                    .inner_margin(egui::Margin::symmetric(8.0, 3.0)),
            )
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("CONSOLE").monospace().strong().color(output()));
                    ui.add_space(8.0);
                    match status.listening {
                        Some(port) => {
                            ui.label(egui::RichText::new("•").color(ok()));
                            ui.label(
                                egui::RichText::new(format!("listening on 127.0.0.1:{port}"))
                                    .monospace()
                                    .small()
                                    .color(info()),
                            );
                        }
                        None => {
                            ui.label(egui::RichText::new("○").color(info()));
                            ui.label(
                                egui::RichText::new("channel closed: only this console can script")
                                    .monospace()
                                    .small()
                                    .color(info()),
                            );
                        }
                    }
                    ui.label(
                        egui::RichText::new(format!("· {} requests", status.requests))
                            .monospace()
                            .small()
                            .color(info()),
                    );
                    if status.active {
                        ui.label(
                            egui::RichText::new("⚡ a script is acting now")
                                .monospace()
                                .small()
                                .color(remote()),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").on_hover_text("Hide the console · `").clicked() {
                            self.open = false;
                        }
                        if ui.small_button("Clear").clicked() {
                            self.lines.clear();
                        }
                        if ui
                            .small_button("Copy all")
                            .on_hover_text("Copy the whole scrollback")
                            .clicked()
                        {
                            let all: Vec<&str> = self.lines.iter().map(|l| l.text.as_str()).collect();
                            ui.output_mut(|o| o.copied_text = all.join("\n"));
                        }
                        if ui
                            .small_button("Run script…")
                            .on_hover_text("Run a file of console commands, or a JSON request file")
                            .clicked()
                        {
                            action = Action::RunFile;
                        }
                        if ui.small_button("Help").clicked() {
                            action = Action::Run("help".into());
                        }
                    });
                });
            });

        // --- prompt -------------------------------------------------------
        let id = ui.make_persistent_id("console-input");
        let focused = ui.memory(|m| m.has_focus(id));
        if focused {
            // Taken before the text field sees them: Up and Down walk the
            // history, and Tab completes rather than moving focus.
            let (up, down, tab) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
                )
            });
            if up {
                self.history.up(&mut self.input);
                self.cursor_to_end = true;
            }
            if down {
                self.history.down(&mut self.input);
                self.cursor_to_end = true;
            }
            if tab {
                self.complete();
            }
        }
        if std::mem::take(&mut self.cursor_to_end) {
            if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
                let end = egui::text::CCursor::new(self.input.chars().count());
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), id);
            }
        }

        egui::TopBottomPanel::bottom("console-prompt")
            .frame(egui::Frame::none().inner_margin(egui::Margin {
                left: 10.0,
                right: 6.0,
                top: 5.0,
                bottom: 5.0,
            }))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("›").monospace().strong().color(input_colour()));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.input)
                            .id(id)
                            .font(egui::TextStyle::Monospace)
                            .text_color(output())
                            .frame(false)
                            .lock_focus(true)
                            .desired_width(f32::INFINITY)
                            .hint_text("type a command — help lists them"),
                    );
                    let entered =
                        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if entered {
                        let line = std::mem::take(&mut self.input);
                        if !line.trim().is_empty() {
                            self.history.push(line.trim());
                            action = Action::Run(line.trim().to_string());
                        }
                        // Keep typing: a console that drops focus after
                        // every command is a form, not a console.
                        self.focus = true;
                    }
                    if std::mem::take(&mut self.focus) {
                        response.request_focus();
                    }
                });
            });

        // --- scrollback ---------------------------------------------------
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().inner_margin(egui::Margin {
                left: 10.0,
                right: 6.0,
                top: 4.0,
                bottom: 0.0,
            }))
            .show_inside(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt("console-scrollback")
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show_rows(ui, row_height, self.lines.len(), |ui, rows| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for index in rows {
                            let line = &self.lines[index];
                            let text = if line.text.is_empty() { " " } else { line.text.as_str() };
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(text).monospace().color(line.kind.colour()),
                                )
                                .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        }
                    });
            });

        action
    }

    /// Tab: finish the first word if there is one way to, list the options if
    /// there are several.
    fn complete(&mut self) {
        let completion = shell::complete(&self.input, &tequila_scene::command::Command::CATALOG);
        if let Some(line) = completion.line {
            self.input = line;
            self.cursor_to_end = true;
        }
        // Several to a line, not one long line: the scrollback does not wrap,
        // and forty command names in a row run off the edge of any window.
        for row in completion.candidates.chunks(6) {
            self.push(Kind::Info, &row.join("  "));
        }
    }
}

/// Run one console line against the application.
///
/// Returns whether it succeeded, so `run` can stop a script at its first
/// failure rather than carrying on against a scene that is no longer the one
/// the script was written for.
pub fn run(app: &mut TequilaApp, line: &str) -> bool {
    app.console.push(Kind::Input, &format!("› {line}"));
    run_nested(app, line, 0)
}

fn run_nested(app: &mut TequilaApp, line: &str, depth: usize) -> bool {
    let parsed = match shell::parse(line) {
        Ok(parsed) => parsed,
        Err(message) => {
            app.console.push(Kind::Error, &message);
            return false;
        }
    };
    match parsed {
        Parsed::Empty => true,
        Parsed::Local(local) => local_command(app, local, depth),
        Parsed::Request { json, view } => {
            let request = match parse_request(&json) {
                Ok(request) => request,
                Err(error) => {
                    app.console.push(Kind::Error, &error.to_string());
                    return false;
                }
            };
            match crate::channel::handle(app, &request) {
                Reply::Done => {
                    app.console.push(Kind::Success, "ok");
                    app.console.last_answer = "ok".into();
                    true
                }
                Reply::Value(value) => {
                    let text = shell::render(&view, &value);
                    app.console.push(Kind::Output, &text);
                    app.console.last_answer = text;
                    true
                }
                Reply::Failed(message) => {
                    app.console.push(Kind::Error, &message);
                    app.console.last_answer = message;
                    false
                }
            }
        }
    }
}

fn local_command(app: &mut TequilaApp, local: Local, depth: usize) -> bool {
    let console = &mut app.console;
    match local {
        Local::Help(None) => {
            let width = shell::HELP.iter().map(|(usage, _)| usage.len()).max().unwrap_or(0);
            for (usage, what) in shell::HELP {
                console.push(Kind::Output, &format!("  {usage:<width$}   {what}"));
            }
            console.push(
                Kind::Info,
                "Every command here is also a request an external script can send; `api` describes them.",
            );
        }
        Local::Help(Some(verb)) => {
            let verb = verb.to_ascii_lowercase();
            let matching: Vec<_> = shell::HELP
                .iter()
                .filter(|(usage, _)| {
                    usage.split(|c: char| !c.is_alphanumeric() && c != '.').any(|w| w == verb)
                })
                .collect();
            if matching.is_empty() {
                console.push(Kind::Error, &format!("no help for `{verb}` — type help for everything"));
                return false;
            }
            for (usage, what) in matching {
                console.push(Kind::Output, &format!("  {usage}   {what}"));
            }
        }
        Local::Clear => console.lines.clear(),
        Local::History => {
            if console.history.entries.is_empty() {
                console.push(Kind::Info, "nothing typed yet");
            }
            let lines: Vec<String> = console
                .history
                .entries
                .iter()
                .enumerate()
                .map(|(i, line)| format!("{:>4}  {line}", i + 1))
                .collect();
            for line in lines {
                console.push(Kind::Output, &line);
            }
        }
        Local::Copy => {
            if console.last_answer.is_empty() {
                console.push(Kind::Info, "nothing to copy yet");
            } else {
                console.to_clipboard = Some(console.last_answer.clone());
                console.push(Kind::Info, "copied the last answer");
            }
        }
        Local::Examples => {
            for (i, (title, _)) in shell::examples().iter().enumerate() {
                console.push(Kind::Output, &format!("  {:>2}  {title}", i + 1));
            }
            console.push(Kind::Info, "example <n> puts one at the prompt to read, edit and run");
        }
        Local::Example(n) => match shell::examples().get(n.wrapping_sub(1)) {
            Some((title, request)) => {
                console.push(Kind::Info, &format!("# {title}"));
                console.input = request.clone();
                console.cursor_to_end = true;
                console.focus = true;
            }
            None => {
                console.push(Kind::Error, &format!("there is no example {n} — examples lists them"));
                return false;
            }
        },
        Local::Run(path) => return run_file(app, &path, depth),
    }
    true
}

fn run_file(app: &mut TequilaApp, path: &str, depth: usize) -> bool {
    if depth >= NESTING {
        app.console.push(Kind::Error, "scripts are nested too deeply — does one run itself?");
        return false;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            app.console.push(Kind::Error, &format!("could not read {path}: {error}"));
            return false;
        }
    };
    let lines = match shell::script_lines(&text) {
        Ok(lines) => lines,
        Err(error) => {
            app.console.push(Kind::Error, &format!("{path}: {error}"));
            return false;
        }
    };
    let total = lines.len();
    // The whole script is one undo step, and a script that fails part-way is
    // taken back: its first half was written for a scene the second half
    // never got to finish, and leaving it applied leaves a state nobody
    // chose. Fix the line and run it again.
    let camera = app.project.camera().clone();
    app.project.begin_group();
    for (done, line) in lines.iter().enumerate() {
        app.console.push(Kind::Input, &format!("  {line}"));
        if !run_nested(app, line, depth + 1) {
            let undone = app.project.abandon_group();
            if *app.project.camera() != camera {
                app.camera.adopt(app.project.camera());
            }
            app.console.push(
                Kind::Error,
                &format!(
                    "stopped at line {} of {total}; the script was taken back ({undone} change{} undone)",
                    done + 1,
                    if undone == 1 { "" } else { "s" }
                ),
            );
            return false;
        }
    }
    app.project.end_group();
    app.console.push(Kind::Success, &format!("ran {total} lines from {path} — one undo takes it all back"));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_walks_back_and_returns_the_draft() {
        let mut history = History::default();
        history.push("scene");
        history.push("nodes");
        let mut input = String::from("sel");
        history.up(&mut input);
        assert_eq!(input, "nodes");
        history.up(&mut input);
        assert_eq!(input, "scene");
        // Past the oldest: stays there rather than wrapping.
        history.up(&mut input);
        assert_eq!(input, "scene");
        history.down(&mut input);
        assert_eq!(input, "nodes");
        // Past the newest: the half-typed line comes back.
        history.down(&mut input);
        assert_eq!(input, "sel");
    }

    #[test]
    fn a_repeated_command_is_one_history_entry() {
        let mut history = History::default();
        history.push("frame");
        history.push("frame");
        assert_eq!(history.entries.len(), 1);
    }

    #[test]
    fn the_scrollback_is_bounded() {
        let mut console = Console::default();
        for i in 0..(KEPT + 10) {
            console.push(Kind::Output, &i.to_string());
        }
        assert_eq!(console.lines.len(), KEPT);
        assert_eq!(console.lines.back().map(|l| l.text.as_str()), Some((KEPT + 9).to_string().as_str()));
    }
}
