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
//!
//! A notice may offer one thing to do next, as a link at its end - a
//! Screenshot's "Open folder". Only the link takes a click, and the notice
//! stays a while longer, and while the pointer is on it, so it can be
//! reached; ignored, it goes as any other does.

use eframe::egui;
use std::time::{Duration, Instant};

/// How long a notice stays before it begins to fade.
const HOLD: Duration = Duration::from_millis(2600);
/// And how long the fade itself takes.
const FADE: Duration = Duration::from_millis(450);
/// How long one with something to do stays, to be reached.
const HOLD_ACTION: Duration = Duration::from_millis(6000);

/// What a notice's link does.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Show this file in the file manager: its folder, opened on it.
    Reveal(std::path::PathBuf),
}

#[derive(Clone, Debug)]
struct Notice {
    text: String,
    kind: Kind,
    raised: Instant,
    /// How many times this same message has arrived.
    count: u32,
    /// A link at its end: what it says, and what it does.
    action: Option<(String, Action)>,
}

impl Notice {
    fn hold(&self) -> Duration {
        if self.action.is_some() { HOLD_ACTION } else { HOLD }
    }
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
    /// A link clicked, for the application to act on.
    chosen: Option<Action>,
}

impl Toasts {
    pub fn info(&mut self, text: impl Into<String>) {
        self.push(text.into(), Kind::Info);
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.push(text.into(), Kind::Warning);
    }

    /// Something happened, with one thing to do next: `label`, a link at
    /// the notice's end, which [`Toasts::take_action`] hands over clicked.
    pub fn info_with(&mut self, text: impl Into<String>, label: impl Into<String>, action: Action) {
        self.notices.push(Notice { text: text.into(), kind: Kind::Info, raised: Instant::now(), count: 1, action: Some((label.into(), action)) });
        if self.notices.len() > 4 {
            self.notices.remove(0);
        }
    }

    /// The link clicked since this was last asked, if one was.
    pub fn take_action(&mut self) -> Option<Action> {
        self.chosen.take()
    }

    fn push(&mut self, text: String, kind: Kind) {
        // Coalesce with the most recent notice if it says the same thing,
        // rather than appending a duplicate.
        if let Some(last) = self.notices.last_mut() {
            if last.text == text && last.kind == kind && last.action.is_none() {
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
            action: None,
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
            .retain(|n| now.duration_since(n.raised) < n.hold() + FADE);
        if self.notices.is_empty() {
            return false;
        }
        // Frames through a fade, and one when the next fade begins: the
        // window draws only when asked now, not sixty times a second.
        let until_fade = self.notices.iter().map(|n| (n.raised + n.hold()).saturating_duration_since(now)).min().unwrap_or_default();
        if until_fade.is_zero() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(until_fade);
        }

        // Painted in a foreground layer rather than as a panel: a toast must
        // not take part in layout, or the viewport would resize every time one
        // appeared and the render would restart.
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("toasts"),
        ));

        let mut y = viewport.bottom() - 26.0;
        let mut done = None;
        for (index, notice) in self.notices.iter_mut().enumerate().rev() {
            let age = now.duration_since(notice.raised);
            let hold = notice.hold();
            let alpha = if age < hold {
                1.0
            } else {
                1.0 - (age - hold).as_secs_f32() / FADE.as_secs_f32()
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
            let link = notice.action.as_ref().map(|(label, _)| {
                painter.layout_no_wrap(label.clone(), egui::FontId::proportional(12.5), crate::theme::p().accent.gamma_multiply(alpha))
            });
            let room = link.as_ref().map_or(0.0, |l| l.size().x + 16.0);
            let size = galley.size() + egui::vec2(22.0 + room, 11.0);
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
            let text_at = egui::pos2(rect.left() + 11.0, rect.center().y - galley.size().y * 0.5);
            painter.galley(text_at, galley, egui::Color32::PLACEHOLDER);
            if let (Some(link), Some((_, action))) = (link, &notice.action) {
                let at = egui::Rect::from_min_size(
                    egui::pos2(rect.right() - 11.0 - link.size().x, rect.center().y - link.size().y * 0.5),
                    link.size(),
                );
                // The link alone takes the pointer: an area of its own, the
                // size of its words, over the notice.
                let response = egui::Area::new(egui::Id::new(("toast-action", notice.text.as_str())))
                    .order(egui::Order::Foreground)
                    .fixed_pos(at.min)
                    .show(ctx, |ui| ui.allocate_exact_size(at.size(), egui::Sense::click()).1)
                    .inner;
                let colour = crate::theme::p().accent.gamma_multiply(alpha);
                painter.galley(at.min, link, egui::Color32::PLACEHOLDER);
                if response.hovered() {
                    // Kept while it is pointed at, so it cannot fade away
                    // under the pointer.
                    notice.raised = now;
                    painter.line_segment([at.left_bottom(), at.right_bottom()], egui::Stroke::new(1.0_f32, colour));
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if response.clicked() {
                    self.chosen = Some(action.clone());
                    done = Some(index);
                }
            }
            y -= size.y + 6.0;
        }
        if let Some(index) = done {
            self.notices.remove(index);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A notice on screen asks for the frames it needs: one when its fade
    /// begins, and every frame through the fade, the window now drawing
    /// only when asked.
    #[test]
    fn a_notice_asks_for_its_own_frames() {
        let ctx = egui::Context::default();
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = asked.clone();
        ctx.set_request_repaint_callback(move |info| record.lock().unwrap().push(info.delay));
        let mut toasts = Toasts::default();
        // egui's own first frames ask for more; settled first.
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |_| {});
        }
        asked.lock().unwrap().clear();
        toasts.info("Hid 1 part");
        let frame = |toasts: &mut Toasts| {
            let mut shown = false;
            let _ = ctx.run(egui::RawInput::default(), |ctx| shown = toasts.show(ctx, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))));
            shown
        };
        assert!(frame(&mut toasts));
        let delays: Vec<Duration> = std::mem::take(&mut *asked.lock().unwrap());
        assert!(delays.iter().any(|d| *d > Duration::from_millis(100) && *d < Duration::from_secs(30)), "held, the next frame is asked for when its fade begins: {delays:?}");
        let hold = toasts.notices[0].hold();
        toasts.notices[0].raised -= hold + Duration::from_millis(10);
        assert!(frame(&mut toasts));
        let delays: Vec<Duration> = std::mem::take(&mut *asked.lock().unwrap());
        assert!(delays.contains(&Duration::ZERO), "fading, every frame: {delays:?}");
    }

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

    /// A notice with something to do stays longer than one without, and is
    /// a notice of its own even when the words are the same: its link must
    /// not be lost into an earlier notice that has none.
    #[test]
    fn a_notice_with_a_link_stays_longer_and_keeps_it() {
        let mut toasts = Toasts::default();
        toasts.info("Saved Untitled.png");
        toasts.info_with("Saved Untitled.png", "Open folder", Action::Reveal("Untitled.png".into()));
        toasts.info("Saved Untitled.png");
        assert_eq!(toasts.notices.len(), 3, "a link was coalesced away");
        assert_eq!(toasts.notices[1].action, Some(("Open folder".to_string(), Action::Reveal("Untitled.png".into()))));
        assert!(toasts.notices[1].hold() > toasts.notices[0].hold());
        assert_eq!(toasts.take_action(), None, "nothing was clicked");
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
