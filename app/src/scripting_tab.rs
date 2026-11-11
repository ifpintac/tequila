//! The Scripting tab: what the channel is, and what has been done through it.
//!
//! Two jobs, and the second is the one that matters.
//!
//! The first is access — the port and token a client needs. Typing at the
//! application yourself happens in the console under the viewport
//! ([`crate::console`]). `docs/04` §8 wants everything
//! clickable to be scriptable; this is where a person finds out how.
//!
//! The second is **watching**. A channel that lets something outside the
//! application change a scene is a channel whose activity has to be visible,
//! or the person in front of the screen is the last to know what happened to
//! their work. Every request is logged with where it came from and what it
//! did, the log is in the same place the connection details are, and a light
//! on the toolbar says when something is using it right now. None of that is
//! security — anything with the token can still act — but it is the
//! difference between a tool acting on your behalf and a tool acting behind
//! your back.

use std::collections::VecDeque;

/// How many entries the log keeps.
///
/// Enough that a person who looks up after a minute of scripted work can see
/// all of it, and bounded so a runaway client cannot grow the application's
/// memory without limit. A script that issues more than this in one burst is
/// one whose individual steps were never going to be read anyway.
const KEPT: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Failed,
}

#[derive(Clone)]
pub struct Entry {
    /// Wall clock, for reading.
    pub at: String,
    /// Monotonic, for "is this happening right now".
    pub since: std::time::Instant,
    /// Where it came from: an address, or the console.
    pub origin: String,
    pub summary: String,
    pub outcome: Outcome,
    /// The error, when there was one. A log that says only "failed" makes a
    /// person open a terminal to find out why, which defeats the point of
    /// having put it here.
    pub detail: String,
}

#[derive(Default)]
pub struct Activity {
    entries: VecDeque<Entry>,
    /// Everything ever, not just what is kept, so the count does not lie
    /// after the buffer wraps.
    total: u64,
}

impl Activity {
    pub fn record(&mut self, origin: &str, summary: String, outcome: Outcome, detail: String) {
        self.total += 1;
        if self.entries.len() == KEPT {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry {
            at: clock(),
            since: std::time::Instant::now(),
            origin: origin.to_string(),
            summary,
            outcome,
            detail,
        });
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Newest first, which is the order someone checking on a running script
    /// wants it in.
    pub fn newest_first(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().rev()
    }

    /// Whether something used the channel in the last couple of seconds.
    ///
    /// Drives the toolbar light. Two seconds because a script issuing a
    /// command every frame should read as continuously active rather than
    /// flickering, and one that stopped should go dark quickly enough to be
    /// believed.
    pub fn active(&self) -> bool {
        self.entries
            .back()
            .is_some_and(|e| e.since.elapsed() < std::time::Duration::from_secs(2))
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Local wall-clock time as `HH:MM:SS`, so a stamp matches the clock on the
/// wall rather than UTC.
pub fn clock() -> String {
    crate::clock::time()
}

/// What the tab needs to know that lives elsewhere in the application.
pub struct ChannelStatus {
    pub open: bool,
    pub port: u16,
    pub discovery: std::path::PathBuf,
}

/// What the tab wants the application to do once it has stopped borrowing it.
pub enum Action {
    None,
    OpenConsole,
    ClearLog,
}

/// The tab: how to connect, and what has connected.
///
/// Typing at the application happens in the console under the viewport; this
/// is the reference page beside it.
pub fn show(ui: &mut egui::Ui, status: &ChannelStatus, activity: &Activity) -> Action {
    let mut action = Action::None;
    let p = crate::theme::p();

    // --- what it is -------------------------------------------------------
    // A card saying whether anything can connect, in words first: the
    // address is for whoever is writing the script, and the file's path -
    // a long one, which ran the page wider than its panel - is a click to
    // copy, shown whole when pointed at.
    crate::theme::card(ui, |ui| {
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(egui::Vec2::splat(10.0), egui::Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.0, if status.open { p.success } else { p.text_faint });
            ui.label(egui::RichText::new(if status.open { "Ready for scripts and AI apps" } else { "Not open this session" }).font(crate::theme::strong(crate::theme::BODY)));
        });
        if status.open {
            crate::theme::note(ui, format!("Listening on this computer only, at 127.0.0.1:{}. A script finds it through the connection file.", status.port));
        } else {
            crate::theme::note(
                ui,
                "Nothing outside Tequila can drive it now. The status line says why it could not be opened \
                 (TEQUILA_NO_CHANNEL keeps it closed on purpose). The console still works.",
            );
        }
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            if ui
                .button("Open the console")
                .on_hover_text("Type commands yourself, in the console under the view")
                .clicked()
            {
                action = Action::OpenConsole;
            }
            if ui
                .add_enabled(status.open, egui::Button::new("Copy the connection file's path"))
                .on_hover_text(format!("{}\n\nThe file a script reads the port and its password from.", status.discovery.display()))
                .clicked()
            {
                ui.output_mut(|o| o.copied_text = status.discovery.display().to_string());
            }
            if ui
                .add_enabled(status.open, egui::Button::new("Copy instructions for an AI"))
                .on_hover_text("Paste these into a chat with an assistant that can run code on this computer")
                .clicked()
            {
                ui.output_mut(|o| o.copied_text = instructions(status));
            }
        });
    });

    // --- watching it ------------------------------------------------------
    let total = activity.total();
    let clear = crate::theme::heading_with(ui, "What they did", |ui| {
        let clear = ui.add_enabled(!activity.is_empty(), egui::Button::new("Clear").small()).on_hover_text("Empty this list").clicked();
        ui.label(
            egui::RichText::new(match total {
                0 => String::new(),
                1 => "1 request".to_string(),
                n => format!("{n} requests"),
            })
            .size(crate::theme::BODY - 1.5)
            .color(p.text_dim),
        );
        if activity.active() {
            ui.label(egui::RichText::new("in use now").size(crate::theme::BODY - 1.5).color(p.busy));
        }
        clear
    });
    if clear {
        action = Action::ClearLog;
    }

    if activity.is_empty() {
        crate::theme::note(ui, "Nothing has driven Tequila yet. Every request a script or an AI app makes is listed here as it happens, newest first.");
        return action;
    }

    // One line a request: when, what, and whether it worked; cut short at
    // the panel's edge with the whole of it, and where it came from, when
    // pointed at. A long path ran every line off the panel.
    let font = egui::FontId::proportional(crate::theme::BODY - 0.5);
    for entry in activity.newest_first().take(SHOWN) {
        let failed = entry.outcome == Outcome::Failed;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if response.hovered() {
                painter.rect_filled(rect, 5.0, p.hover.gamma_multiply(0.45));
            }
            painter.circle_filled(egui::pos2(rect.left() + 7.0, rect.center().y), 3.0, if failed { p.danger } else { p.success });
            let time = painter.layout_no_wrap(entry.at.clone(), egui::FontId::monospace(crate::theme::BODY - 2.0), p.text_faint);
            let time_width = time.size().x;
            painter.galley(egui::pos2(rect.right() - 4.0 - time_width, rect.center().y - time.size().y * 0.5), time, egui::Color32::PLACEHOLDER);
            let mut job = egui::text::LayoutJob::simple_singleline(entry.summary.clone(), font.clone(), if failed { p.danger } else { p.text });
            job.wrap = egui::text::TextWrapping { max_width: (rect.width() - 18.0 - time_width - 12.0).max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            let galley = ui.fonts(|f| f.layout_job(job));
            painter.galley(egui::pos2(rect.left() + 18.0, rect.center().y - galley.size().y * 0.5), galley, egui::Color32::PLACEHOLDER);
        }
        response.on_hover_ui(|ui| {
            ui.set_max_width(360.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.label(&entry.summary);
            ui.label(egui::RichText::new(format!("{} · from {}", entry.at, entry.origin)).size(crate::theme::BODY - 1.5).color(p.text_dim));
            if failed && !entry.detail.is_empty() {
                ui.label(egui::RichText::new(&entry.detail).color(p.danger));
            }
        });
        if failed && !entry.detail.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.add(egui::Label::new(egui::RichText::new(&entry.detail).size(crate::theme::BODY - 1.5).color(p.danger)).wrap());
            });
        }
    }

    let kept = activity.newest_first().count();
    if kept > SHOWN {
        crate::theme::note(ui, format!("And {} earlier. The console under the view has them all.", kept - SHOWN));
    }

    action
}

/// How many of the log's entries the page lists: the latest, which are the
/// ones looked for, on a page short enough to scroll.
const SHOWN: usize = 60;

/// What an assistant needs to be told to drive this session.
fn instructions(status: &ChannelStatus) -> String {
    format!(
        "Control this running Tequila session using its local JSON-line TCP API. Read the port, \
         token and PID locally from {} (do not publish the token). Start with \
         {{\"op\":\"query\",\"what\":\"api\"}}, then query context. Include token and a unique \
         integer id in each request. Resolve node/material IDs from the current context; do not \
         guess them. Materials may be shared: unlink a part before changing it if needed. Each \
         apply is undoable separately. Use frame and screenshot to inspect your work; screenshots \
         are deferred. Render is synchronous and blocks the UI. Do not assume a timeout means an \
         edit failed, and never retry mutations blindly. Everything you send is shown to the user \
         in the console under the viewport as it happens. The Python client and examples are in \
         the distribution's python folder.",
        status.discovery.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_bounded_but_the_count_is_not() {
        let mut activity = Activity::default();
        for i in 0..(KEPT + 50) {
            activity.record("test", format!("op {i}"), Outcome::Ok, String::new());
        }
        assert_eq!(activity.newest_first().count(), KEPT);
        // The buffer wrapped; the tally must not pretend otherwise, or a
        // person checking "how much did that script do" gets a number that
        // silently stops counting.
        assert_eq!(activity.total(), (KEPT + 50) as u64);
    }

    #[test]
    fn the_newest_entry_comes_first() {
        let mut activity = Activity::default();
        activity.record("a", "first".into(), Outcome::Ok, String::new());
        activity.record("b", "second".into(), Outcome::Failed, "why".into());
        let entries: Vec<_> = activity.newest_first().collect();
        assert_eq!(entries[0].summary, "second");
        assert_eq!(entries[0].outcome, Outcome::Failed);
        assert_eq!(entries[0].detail, "why");
    }

    /// The toolbar light: on straight after something happened, and honest
    /// about an empty log.
    #[test]
    fn activity_is_reported_only_when_recent() {
        let mut activity = Activity::default();
        assert!(!activity.active(), "an empty log is not activity");
        activity.record("a", "op".into(), Outcome::Ok, String::new());
        assert!(activity.active());
    }
}
