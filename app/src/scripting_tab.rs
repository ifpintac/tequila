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

    // --- what it is -------------------------------------------------------
    if status.open {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("•").color(crate::theme::p().success));
            ui.label(format!("Listening on 127.0.0.1:{}", status.port));
        });
        ui.label(
            egui::RichText::new(format!(
                "A client reads the port and token from {}",
                status.discovery.display()
            ))
            .small()
            .weak(),
        );
    } else {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("•").color(egui::Color32::DARK_GRAY));
            ui.label("The scripting channel is not open this session.");
        });
        ui.label(
            egui::RichText::new(
                "Set TEQUILA_NO_CHANNEL to keep it closed, or check the status \
                 bar for why it could not be opened. The console still works.",
            )
            .small()
            .weak(),
        );
    }

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Open console")
            .on_hover_text("The console under the viewport · `")
            .clicked()
        {
            action = Action::OpenConsole;
        }
        if ui.button("Copy connection file path").clicked() {
            ui.output_mut(|o| o.copied_text = status.discovery.display().to_string());
        }
    });
    if ui
        .button("Copy instructions for an AI assistant")
        .on_hover_text("Paste these into a chat with an assistant that can run code on this machine")
        .clicked()
    {
        ui.output_mut(|o| o.copied_text = instructions(status));
    }

    ui.add_space(8.0);
    ui.separator();

    // --- watching it ------------------------------------------------------
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Activity").strong());
        if activity.active() {
            ui.label(
                egui::RichText::new("in use now")
                    .small()
                    .color(crate::theme::p().busy),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Clear").clicked() {
                action = Action::ClearLog;
            }
            ui.label(
                egui::RichText::new(format!("{} request(s)", activity.total()))
                    .small()
                    .weak(),
            );
        });
    });

    if activity.is_empty() {
        ui.label(
            egui::RichText::new("Nothing has used the channel yet.")
                .small()
                .weak(),
        );
        return action;
    }

    egui::ScrollArea::vertical()
        .id_salt("scripting-activity")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for entry in activity.newest_first() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&entry.at).small().weak());
                    let colour = match entry.outcome {
                        Outcome::Ok => crate::theme::p().success,
                        Outcome::Failed => crate::theme::p().danger,
                    };
                    ui.label(egui::RichText::new(&entry.summary).color(colour));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(&entry.origin).small().weak());
                    });
                });
                if entry.outcome == Outcome::Failed && !entry.detail.is_empty() {
                    ui.label(
                        egui::RichText::new(&entry.detail)
                            .small()
                            .color(crate::theme::p().danger),
                    );
                }
            }
        });

    action
}

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
