//! Help > Test this computer: a few minutes of checks, with their progress,
//! a Cancel, and at the end a short report to copy into the Discord.
//!
//! The checks are the command-line program's (`tequila-cli --self-test`,
//! `crates/tequila-cli/src/self_test.rs`): every engine and device here
//! rendering one small scene, timed and compared with a reference. They run
//! as a process of their own, so a graphics driver that crashes takes the
//! test down and not the application, which starts it again past the check
//! that crashed (`--resume`) - up to a few times - and the report says what
//! crashed and how. The application adds what only it can check: its
//! viewport, which the graphics card draws through OpenGL - which card and
//! driver the window's context is on, and whether the Solid view's shaders
//! build there (`solid_gpu::probe`).
//!
//! Nothing is sent anywhere. The report goes in a folder in the person's
//! home with the pictures, and on the clipboard when they ask.

use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Times the test is started again after it stops unexpectedly.
const RESTARTS: u32 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    /// Waiting a frame for the viewport's check, which runs while drawing.
    Probing,
    Running,
    Done,
}

/// What the test's process said.
enum Said {
    Out(String),
    Err(String),
}

/// A finished check, as the window lists it.
#[derive(Clone, Debug)]
struct Row {
    status: String,
    label: String,
    summary: String,
}

#[derive(Default)]
pub struct SelfTest {
    pub open: bool,
    phase: Phase,
    /// Leave the graphics cards out.
    cpu_only: bool,
    probe: Arc<Mutex<Option<String>>>,
    probing_since: Option<Instant>,
    folder: Option<PathBuf>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    said: Option<Receiver<Said>>,
    total: usize,
    step: Option<(usize, String)>,
    fraction: f32,
    rows: Vec<Row>,
    /// The test said it was done; without it, an exit is a crash.
    done: bool,
    /// The last lines it wrote to its error stream, for a crash's note.
    errors: VecDeque<String>,
    restarts: u32,
    cancelling: Option<Instant>,
    report: Option<String>,
    problem: Option<String>,
    copied: Option<Instant>,
}

impl Drop for SelfTest {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
        }
    }
}

impl SelfTest {
    /// Open the window and start the test, unless it is running already.
    pub fn open_and_start(&mut self) {
        self.open = true;
        if !matches!(self.phase, Phase::Probing | Phase::Running) {
            self.start();
        }
    }

    /// Close the window, stopping the test if it is running.
    pub fn close(&mut self) {
        self.open = false;
        if self.running() {
            self.cancel();
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.phase, Phase::Probing | Phase::Running)
    }

    /// What a script sees: whether the window is open, how far the test has
    /// got, and how it went.
    pub fn state(&self) -> tequila_format::json::Value {
        let mut out = tequila_format::json_write::Object::new();
        let phase = match self.phase {
            Phase::Idle => "idle",
            Phase::Probing | Phase::Running => "running",
            Phase::Done => "done",
        };
        let count = |status: &str| self.rows.iter().filter(|r| r.status == status).count() as f64;
        out.set("open", self.open)
            .set("phase", phase)
            .set("checks", self.total as f64)
            .set("finished", self.rows.len() as f64)
            .set("passed", count("pass"))
            .set("failed", count("fail") + count("crash"))
            .set("restarts", self.restarts as f64);
        out.build()
    }

    fn start(&mut self) {
        let mut fresh = SelfTest::default();
        fresh.open = true;
        fresh.cpu_only = self.cpu_only;
        fresh.folder = Some(crash_folder_sibling());
        fresh.phase = Phase::Probing;
        fresh.probing_since = Some(Instant::now());
        *self = fresh;
    }

    /// Start the test's process: afresh, or carrying on after a crash.
    fn spawn(&mut self, resume: Option<String>) {
        let Some(program) = crate::connect_apps::server_program() else {
            self.finish_with("Tequila's command-line program, which runs the checks, is not beside the application. Reinstalling Tequila puts it back.".into());
            return;
        };
        let folder = self.folder.clone().unwrap_or_else(crash_folder_sibling);
        let viewport = self.probe.lock().ok().and_then(|p| p.clone()).unwrap_or_else(|| "failed: the viewport did not draw".into());
        let mut command = Command::new(program);
        command.arg("--self-test").arg("-o").arg(&folder).arg("--machine").arg("--viewport").arg(&viewport);
        if self.cpu_only {
            command.arg("--cpu-only");
        }
        if let Some(why) = &resume {
            command.arg("--resume").arg("--crashed").arg(why);
        }
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        match command.spawn() {
            Ok(mut child) => {
                let (send, receive) = channel();
                if let Some(out) = child.stdout.take() {
                    read_lines(out, send.clone(), Said::Out);
                }
                if let Some(err) = child.stderr.take() {
                    read_lines(err, send, Said::Err);
                }
                self.stdin = child.stdin.take();
                self.child = Some(child);
                self.said = Some(receive);
                self.done = false;
                self.phase = Phase::Running;
            }
            Err(error) => self.finish_with(format!("The checks could not start: {error}")),
        }
    }

    fn cancel(&mut self) {
        if self.cancelling.is_none() {
            self.cancelling = Some(Instant::now());
            if let Some(stdin) = &mut self.stdin {
                let _ = writeln!(stdin, "cancel");
                let _ = stdin.flush();
            }
        }
    }

    fn finish_with(&mut self, problem: String) {
        self.problem = Some(problem);
        self.phase = Phase::Done;
        self.read_report();
    }

    fn read_report(&mut self) {
        if let Some(folder) = &self.folder {
            self.report = std::fs::read_to_string(folder.join("report.md")).ok();
        }
    }

    /// What the process said since the last frame, and whether it is still
    /// there.
    fn drain(&mut self) {
        let lines: Vec<Said> = self.said.as_ref().map(|said| said.try_iter().collect()).unwrap_or_default();
        for line in lines {
            match line {
                Said::Out(line) => self.heard(&line),
                Said::Err(line) => {
                    if !line.trim().is_empty() {
                        self.errors.push_back(line);
                        while self.errors.len() > 6 {
                            self.errors.pop_front();
                        }
                    }
                }
            }
        }
    }

    fn poll(&mut self) {
        self.drain();
        // Stopping, it is given a while to write what it has, then ended.
        if self.cancelling.is_some_and(|t| t.elapsed() > Duration::from_secs(20)) {
            if let Some(child) = &mut self.child {
                let _ = child.kill();
            }
        }
        let Some(child) = &mut self.child else { return };
        let Ok(Some(status)) = child.try_wait() else { return };
        // Whatever it wrote last, before deciding what its going means.
        std::thread::sleep(Duration::from_millis(50));
        self.drain();
        self.child = None;
        self.stdin = None;
        self.said = None;
        if self.done || self.cancelling.is_some() {
            self.phase = Phase::Done;
            self.read_report();
            return;
        }
        let how = exit_text(&status);
        let why = match panic_message(self.errors.iter().map(String::as_str)) {
            Some(panic) => format!("{how}: {panic}"),
            None => how,
        };
        if self.restarts < RESTARTS {
            self.restarts += 1;
            self.errors.clear();
            self.spawn(Some(why));
        } else {
            self.finish_with(format!("The checks stopped unexpectedly {} times ({why}).", self.restarts + 1));
        }
    }

    fn heard(&mut self, line: &str) {
        let line = line.trim_end();
        if let Some(count) = line.strip_prefix("@total ") {
            self.total = count.trim().parse().unwrap_or(0);
        } else if let Some(step) = line.strip_prefix("@step ") {
            let (index, label) = step.split_once(' ').unwrap_or((step, ""));
            self.step = Some((index.parse().unwrap_or(0), label.to_string()));
            self.fraction = 0.0;
        } else if let Some(fraction) = line.strip_prefix("@progress ") {
            self.fraction = fraction.trim().parse().unwrap_or(0.0);
        } else if let Some(result) = line.strip_prefix("@result ") {
            let (status, rest) = result.split_once(' ').unwrap_or((result, ""));
            let (label, summary) = rest.split_once('\t').unwrap_or((rest, ""));
            // A check carried over from before a crash is listed once.
            self.rows.retain(|r| r.label != label);
            self.rows.push(Row { status: status.into(), label: label.into(), summary: summary.into() });
        } else if line.starts_with("@done") {
            self.done = true;
            self.step = None;
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        // Followed with the window closed too, to its end.
        self.poll();
        if !self.open {
            if self.running() {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
            return;
        }
        if self.phase == Phase::Probing {
            let probed = self.probe.lock().ok().is_some_and(|p| p.is_some());
            if probed || self.probing_since.is_some_and(|t| t.elapsed() > Duration::from_secs(3)) {
                self.spawn(None);
            }
        }
        if self.running() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let mut open = self.open;
        egui::Window::new("Test this computer")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(640.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| self.contents(ui));
        if !open {
            self.close();
        }
    }

    fn contents(&mut self, ui: &mut egui::Ui) {
        let p = crate::theme::p();
        ui.label(
            "Renders a small scene with every engine and graphics card here, times each and checks the picture against a reference, \
             then tries the denoisers and the viewport. A minute or two; longer the first time a graphics card is used. \
             Nothing is sent anywhere: you get a report to copy.",
        );
        ui.add_space(6.0);
        // The viewport's check runs as the window draws, on its OpenGL.
        if self.phase == Phase::Probing {
            let probe = self.probe.clone();
            let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 1.0), egui::Sense::hover());
            ui.painter().add(egui::PaintCallback {
                rect,
                callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |_, painter| {
                    if let Ok(mut slot) = probe.lock() {
                        if slot.is_none() {
                            *slot = Some(crate::solid_gpu::probe(painter.gl()));
                        }
                    }
                })),
            });
        }
        match self.phase {
            Phase::Idle => {
                ui.horizontal(|ui| {
                    if ui.button("Start the test").clicked() {
                        self.start();
                    }
                    ui.checkbox(&mut self.cpu_only, "Processor only").on_hover_text("Leave the graphics cards out");
                });
            }
            Phase::Probing | Phase::Running => {
                let (index, label) = self.step.clone().unwrap_or((0, "Looking at the computer".into()));
                let done = self.rows.len() as f32;
                let overall = if self.total > 0 { ((done + self.fraction.clamp(0.0, 1.0)) / self.total as f32).min(1.0) } else { 0.0 };
                let text = if self.total > 0 && index > 0 { format!("{index} of {}: {label}", self.total) } else { label };
                ui.add(egui::ProgressBar::new(overall).text(text));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let stopping = self.cancelling.is_some();
                    if ui.add_enabled(!stopping, egui::Button::new(if stopping { "Stopping..." } else { "Cancel" })).clicked() {
                        self.cancel();
                    }
                    if self.restarts > 0 {
                        ui.label(egui::RichText::new(format!("Started again after a crash ({}).", self.restarts)).color(p.danger));
                    }
                });
            }
            Phase::Done => {}
        }
        if !self.rows.is_empty() {
            ui.add_space(6.0);
            egui::Grid::new("self-test-rows").num_columns(3).spacing(egui::vec2(10.0, 4.0)).show(ui, |ui| {
                for row in &self.rows {
                    let (icon, colour) = match row.status.as_str() {
                        "pass" => (crate::icons::Icon::Check, p.success),
                        "fail" | "crash" => (crate::icons::Icon::Close, p.danger),
                        _ => (crate::icons::Icon::More, p.text_faint),
                    };
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    crate::icons::paint(ui.painter(), rect, icon, colour);
                    ui.label(&row.label);
                    // In full in the report.
                    let summary: String = if row.summary.chars().count() > 110 {
                        row.summary.chars().take(108).chain(std::iter::once('…')).collect()
                    } else {
                        row.summary.clone()
                    };
                    ui.label(egui::RichText::new(summary).color(p.text_dim)).on_hover_text(&row.summary);
                    ui.end_row();
                }
            });
        }
        if self.phase != Phase::Done {
            return;
        }
        ui.add_space(8.0);
        if let Some(problem) = &self.problem {
            ui.label(egui::RichText::new(problem).color(p.danger));
            ui.add_space(4.0);
        }
        let failed = self.rows.iter().filter(|r| r.status == "fail" || r.status == "crash").count();
        let passed = self.rows.iter().filter(|r| r.status == "pass").count();
        let verdict = if self.cancelling.is_some() {
            "Stopped. The report has what was done.".to_string()
        } else if failed > 0 {
            format!("{failed} of the checks failed. The report says which, and how; posting it helps us fix it.")
        } else {
            format!("All {passed} checks that could run here passed.")
        };
        ui.label(egui::RichText::new(verdict).strong());
        if let Some(report) = self.report.clone() {
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                let mut text = report.as_str();
                ui.add(egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Copy the report").on_hover_text("Ready to paste into Discord").clicked() {
                    ui.ctx().copy_text(report.clone());
                    self.copied = Some(Instant::now());
                }
                if let Some(folder) = &self.folder {
                    if ui.button("Open the folder").on_hover_text("The report and the pictures").clicked() {
                        crate::library_tab::open_folder(folder);
                    }
                }
                if ui.button("Run again").clicked() {
                    self.start();
                }
                if self.copied.is_some_and(|t| t.elapsed().as_secs_f32() < 3.0) {
                    ui.label(egui::RichText::new("Copied").color(p.accent));
                    ui.ctx().request_repaint_after(Duration::from_millis(500));
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label("Paste it into the");
                ui.hyperlink_to("Tequila Discord", crate::start_screen::DISCORD);
                ui.label("so we know how Tequila does on computers like yours.");
            });
        } else if ui.button("Run again").clicked() {
            self.start();
        }
    }
}

/// Where a run's report and pictures go: `.tequila/self-test/<when>` in
/// the person's home, beside the crash reports.
fn crash_folder_sibling() -> PathBuf {
    let crashes = crate::crash::folder();
    let base = match crashes.parent() {
        Some(parent) if crashes.file_name().is_some_and(|n| n == "crashes") => parent.join("self-test"),
        _ => crashes.join("self-test"),
    };
    base.join(format!("{} {}", tequila_core::clock::date(), tequila_core::clock::time_for_filename()))
}

fn read_lines<R: std::io::Read + Send + 'static>(from: R, to: Sender<Said>, wrap: fn(String) -> Said) {
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(from).lines() {
            let Ok(line) = line else { break };
            if to.send(wrap(line)).is_err() {
                break;
            }
        }
    });
}

/// What a panic said, from the last lines the test wrote to its error
/// stream: the line naming where, and the message after it. Cycles' own
/// chatter there (devices it could not open) says nothing about a crash.
fn panic_message<'a>(lines: impl Iterator<Item = &'a str>) -> Option<String> {
    let lines: Vec<&str> = lines.collect();
    let at = lines.iter().rposition(|l| l.contains("panicked at"))?;
    let message = lines.get(at + 1).map(|m| m.trim()).filter(|m| !m.is_empty() && !m.starts_with("note:"));
    Some(match message {
        Some(message) => format!("{message} ({})", lines[at].trim()),
        None => lines[at].trim().to_string(),
    })
}

/// How a process ended, in a few words: a Windows exception code in hex,
/// as people search for it, or the signal on a Mac.
fn exit_text(status: &std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("ended by signal {signal}");
        }
    }
    match status.code() {
        Some(code) if code < 0 || code > 0xffff => format!("exit code 0x{:08x}", code as u32),
        Some(code) => format!("exit code {code}"),
        None => "ended".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test's lines move the window along: the count, each step, its
    /// progress, each result - a check carried over after a crash listed
    /// once - and the end.
    #[test]
    fn the_window_follows_what_the_test_says() {
        let mut test = SelfTest::default();
        for line in ["@total 4", "@step 1 Tequila · processor", "@progress 0.500"] {
            test.heard(line);
        }
        assert_eq!(test.total, 4);
        assert_eq!(test.step, Some((1, "Tequila · processor".into())));
        assert_eq!(test.fraction, 0.5);
        test.heard("@result pass Tequila · processor\t1.2 s · 0.9% (max 3%)");
        test.heard("@result crash Cycles · HIP · Radeon RX 7900 XTX\tthe test stopped during this check");
        test.heard("@result crash Cycles · HIP · Radeon RX 7900 XTX\tthe test stopped during this check: exit code 0xc0000005");
        assert_eq!(test.rows.len(), 2);
        assert_eq!(test.rows[1].summary, "the test stopped during this check: exit code 0xc0000005");
        assert!(!test.done);
        test.heard("@done fail");
        assert!(test.done);
        let state = test.state();
        assert_eq!(state.get("passed").and_then(|v| v.as_f64()), Some(1.0));
        assert_eq!(state.get("failed").and_then(|v| v.as_f64()), Some(1.0));
    }

    #[test]
    fn a_panic_is_told_from_cycles_chatter() {
        let chatter = ["cycles D:/a/cycles/device.cpp:69: HIPEW initialization failed: Error opening HIP dynamic library"];
        assert_eq!(panic_message(chatter.into_iter()), None);
        let panic = [chatter[0], "thread 'main' panicked at crates/tequila-cli/src/self_test.rs:10:5:", "index out of bounds", "note: run with `RUST_BACKTRACE=1`"];
        assert_eq!(
            panic_message(panic.into_iter()).as_deref(),
            Some("index out of bounds (thread 'main' panicked at crates/tequila-cli/src/self_test.rs:10:5:)")
        );
    }

    #[test]
    fn a_crash_is_said_as_people_search_for_it() {
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            assert_eq!(exit_text(&std::process::ExitStatus::from_raw(0xc000_0005)), "exit code 0xc0000005");
            assert_eq!(exit_text(&std::process::ExitStatus::from_raw(3)), "exit code 3");
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(exit_text(&std::process::ExitStatus::from_raw(11)), "ended by signal 11");
        }
    }
}
