//! Transient notices, bottom-centre (`docs/04-UX-SPEC.md` §2.2).
//!
//! The spec's rule is the important part: **never a modal dialog for these**.
//! "Rebuilding BVH", "fell back to CPU", "hid 4 parts" are all things the
//! application did, not questions it is asking, and a dialog for something
//! that needs no answer costs a click and a train of thought every time.
//!
//! The other half of the rule is that a toast must be *ignorable*. It sits
//! away from the pointer, it never takes focus, it never intercepts a click,
//! and it leaves on its own. Anything that fails those is a dialog wearing a
//! different shape.
//!
//! Identical messages coalesce rather than stacking: an action repeated five
//! times in a second is one thing that happened five times, and five copies
//! sliding up the screen is the sort of detail that makes an application feel
//! like it is shouting.

use eframe::egui;
use std::time::{Duration, Instant};

/// How long a notice stays before it begins to fade.
const HOLD: Duration = Duration::from_millis(2600);
/// And how long the fade itself takes.
const FADE: Duration = Duration::from_millis(450);

#[derive(Clone, Debug)]
struct Notice {
    text: String,
    kind: Kind,
    raised: Instant,
    /// How many times this same message has arrived.
    count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Something happened. The overwhelming majority.
    Info,
    /// Something did not work, or worked in a way the user should know about.
    /// Still not a dialog: if it needed an answer it would not be a toast.
    Warning,
}

#[derive(Debug, Default)]
pub struct Toasts {
    notices: Vec<Notice>,
}

impl Toasts {
    pub fn info(&mut self, text: impl Into<String>) {
        self.push(text.into(), Kind::Info);
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.push(text.into(), Kind::Warning);
    }

    fn push(&mut self, text: String, kind: Kind) {
        // Coalesce with the most recent notice if it says the same thing,
        // rather than appending a duplicate.
        if let Some(last) = self.notices.last_mut() {
            if last.text == text && last.kind == kind {
                last.count += 1;
                last.raised = Instant::now();
                return;
            }
        }
        self.notices.push(Notice {
            text,
            kind,
            raised: Instant::now(),
            count: 1,
        });
        // A cap, because a runaway caller should degrade into "the last few
        // things that happened" rather than into a wall.
        if self.notices.len() > 4 {
            self.notices.remove(0);
        }
    }

    /// Paint whatever is still live. Returns true if anything is on screen, so
    /// the caller knows to keep repainting through a fade.
    pub fn show(&mut self, ctx: &egui::Context, viewport: egui::Rect) -> bool {
        let now = Instant::now();
        self.notices
            .retain(|n| now.duration_since(n.raised) < HOLD + FADE);
        if self.notices.is_empty() {
            return false;
        }

        // Painted in a foreground layer rather than as a panel: a toast must
        // not take part in layout, or the viewport would resize every time one
        // appeared and the render would restart.
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("toasts"),
        ));

        let mut y = viewport.bottom() - 26.0;
        for notice in self.notices.iter().rev() {
            let age = now.duration_since(notice.raised);
            let alpha = if age < HOLD {
                1.0
            } else {
                1.0 - (age - HOLD).as_secs_f32() / FADE.as_secs_f32()
            }
            .clamp(0.0, 1.0);

            let text = if notice.count > 1 {
                format!("{}  ×{}", notice.text, notice.count)
            } else {
                notice.text.clone()
            };
            let galley = painter.layout_no_wrap(
                text,
                egui::FontId::proportional(12.5),
                crate::theme::p().text.gamma_multiply(alpha),
            );
            let size = galley.size() + egui::vec2(22.0, 11.0);
            let rect = egui::Rect::from_center_size(
                egui::pos2(viewport.center().x, y - size.y * 0.5),
                size,
            );
            let (fill, edge) = match notice.kind {
                Kind::Info => (crate::theme::p().raised, crate::theme::p().line_strong),
                Kind::Warning => (crate::theme::p().raised, crate::theme::p().busy),
            };
            painter.rect(
                rect,
                egui::Rounding::same(10.0),
                fill.gamma_multiply(alpha * 0.94),
                egui::Stroke::new(1.0_f32, edge.gamma_multiply(alpha)),
            );
            painter.galley(
                rect.center() - galley.size() * 0.5,
                galley,
                egui::Color32::PLACEHOLDER,
            );
            y -= size.y + 6.0;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same message twice is one notice that happened twice, not two
    /// notices. Five copies of "Hid 1 part" sliding up the screen is the sort
    /// of thing that makes an application feel like it is shouting.
    #[test]
    fn identical_messages_coalesce() {
        let mut toasts = Toasts::default();
        toasts.info("Hid 1 part");
        toasts.info("Hid 1 part");
        toasts.info("Hid 1 part");
        assert_eq!(toasts.notices.len(), 1);
        assert_eq!(toasts.notices[0].count, 3);
    }

    /// But different ones do not, or the second thing that happened is lost.
    #[test]
    fn different_messages_stack() {
        let mut toasts = Toasts::default();
        toasts.info("Hid 1 part");
        toasts.info("Unhid 4 parts");
        assert_eq!(toasts.notices.len(), 2);
    }

    /// A warning and an info with the same words are different notices: the
    /// warning is the one somebody has to read.
    #[test]
    fn a_warning_does_not_coalesce_into_an_info() {
        let mut toasts = Toasts::default();
        toasts.info("Rendering");
        toasts.warn("Rendering");
        assert_eq!(toasts.notices.len(), 2);
    }

    /// A runaway caller degrades into "the last few things that happened"
    /// rather than into an unbounded wall of text.
    #[test]
    fn the_queue_is_bounded() {
        let mut toasts = Toasts::default();
        for i in 0..50 {
            toasts.info(format!("thing {i}"));
        }
        assert!(toasts.notices.len() <= 4, "{} notices", toasts.notices.len());
        assert_eq!(toasts.notices.last().unwrap().text, "thing 49");
    }
}
