//! How long the viewport takes to answer, and a made-up mouse to ask it.
//!
//! A camera move is asked for on the window's thread and answered by the
//! render thread's picture, which the window then puts up. What the hand
//! feels is the time between the two, above all at the start of a move,
//! when the view has been still and the renderer is deep in a still
//! picture's work. [`Answers`] times it for every picture that answers a
//! new request - `ui: asked to shown` in `TEQUILA_PROFILE`, and the first
//! after a pause as `ui: move start to shown` - and `TEQUILA_VIEW_TRACE`
//! prints each. [`ScriptedDrag`] is a drag across the viewport given by a
//! script (`{"op":"ui","drag":...}`), pressed, carried and released a frame
//! at a time as a mouse's would be, so the same numbers can be had from an
//! orbit, a pan or a zoom with the mouse without a hand on one.

use std::time::{Duration, Instant};

/// How long the camera has to have been still for the next move to count
/// as a start: longer than the pause between two pictures of an orbit,
/// short enough that a still view is still refining.
const STILL_BEFORE_A_START: Duration = Duration::from_millis(300);

/// The timing of pictures against the requests they answer.
#[derive(Default)]
pub struct Answers {
    /// The camera last asked for, and when it changed.
    camera: Option<tequila_render_core::camera::Camera>,
    changed_at: Option<Instant>,
    /// When the move under way began after a pause, until a picture of it
    /// has been shown.
    start: Option<Instant>,
    /// The request whose picture was shown last.
    shown: Option<Instant>,
}

fn tracing() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("TEQUILA_VIEW_TRACE").is_some())
}

/// A line of `TEQUILA_VIEW_TRACE`, stamped with the seconds since the first.
pub fn trace(what: std::fmt::Arguments) {
    if tracing() {
        static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let since = ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64();
        eprintln!("view: {since:.4} {what}");
    }
}

/// A frame of the window that took `ms`: a slow one is traced.
pub fn frame_took(ms: f64) {
    if ms > 25.0 {
        trace(format_args!("slow frame {ms:.1} ms"));
    }
}

impl Answers {
    /// A request went to the render thread at `at` with `camera`.
    pub fn asked(&mut self, camera: tequila_render_core::camera::Camera, at: Instant) {
        if self.camera == Some(camera) {
            return;
        }
        let still = self.changed_at.is_none_or(|then| at.duration_since(then) >= STILL_BEFORE_A_START);
        if self.camera.is_some() && still {
            self.start = Some(at);
            trace(format_args!("a move starts"));
        }
        self.camera = Some(camera);
        self.changed_at = Some(at);
    }

    /// A picture that answers the request made at `asked` is being put up.
    pub fn shown(&mut self, asked: Option<Instant>, size: (usize, usize), samples: u32, denoised: bool) {
        let Some(asked) = asked else { return };
        let ms = asked.elapsed().as_secs_f64() * 1000.0;
        let new = self.shown != Some(asked);
        let mut first = false;
        if new {
            self.shown = Some(asked);
            crate::profile::record("ui: asked to shown", ms);
            // Timed from the move's start, not from the request answered:
            // a renderer busy when the move began answers a later one.
            if let Some(start) = self.start.filter(|start| asked >= *start) {
                first = true;
                self.start = None;
                crate::profile::record("ui: move start to shown", start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        trace(format_args!(
            "shown {}x{} samples {samples}{} {} {ms:.1} ms after asking{}",
            size.0,
            size.1,
            if denoised { " denoised" } else { "" },
            if new { "new" } else { "again" },
            if first { ", the move's first" } else { "" }
        ));
    }
}

/// A drag across the viewport that a script asked for.
#[derive(Clone, Debug)]
pub struct ScriptedDrag {
    from: egui::Pos2,
    to: egui::Pos2,
    seconds: f32,
    button: egui::PointerButton,
    modifiers: egui::Modifiers,
    /// When the button went down, once it has.
    pressed: Option<Instant>,
}

impl ScriptedDrag {
    pub fn new(drag: &tequila_scripting::protocol::PointerDrag) -> ScriptedDrag {
        let button = match drag.button.as_str() {
            "right" => egui::PointerButton::Secondary,
            "middle" => egui::PointerButton::Middle,
            _ => egui::PointerButton::Primary,
        };
        let modifiers = egui::Modifiers { alt: drag.alt, shift: drag.shift, command: drag.command, mac_cmd: drag.command && cfg!(target_os = "macos"), ctrl: drag.command && !cfg!(target_os = "macos") };
        ScriptedDrag {
            from: egui::pos2(drag.from[0], drag.from[1]),
            to: egui::pos2(drag.to[0], drag.to[1]),
            seconds: drag.seconds,
            button,
            modifiers,
            pressed: None,
        }
    }

    /// This frame's events over a viewport at `rect`, as a mouse would send
    /// them: the press where it starts, then the pointer where it has got
    /// to, then the release where it ends. `false` once it has ended.
    pub fn frame(&mut self, rect: egui::Rect, now: Instant, raw: &mut egui::RawInput) -> bool {
        let at = |place: egui::Pos2| rect.min + egui::vec2(place.x * rect.width(), place.y * rect.height());
        raw.modifiers = raw.modifiers | self.modifiers;
        let modifiers = raw.modifiers;
        let Some(pressed) = self.pressed else {
            trace(format_args!("drag pressed"));
            let pos = at(self.from);
            raw.events.push(egui::Event::PointerMoved(pos));
            raw.events.push(egui::Event::PointerButton { pos, button: self.button, pressed: true, modifiers });
            self.pressed = Some(now);
            return true;
        };
        let t = if self.seconds > 0.0 { (now.duration_since(pressed).as_secs_f32() / self.seconds).min(1.0) } else { 1.0 };
        let pos = at(self.from + (self.to - self.from) * t);
        raw.events.push(egui::Event::PointerMoved(pos));
        if t < 1.0 {
            return true;
        }
        raw.events.push(egui::Event::PointerButton { pos, button: self.button, pressed: false, modifiers });
        trace(format_args!("drag released"));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up drag presses where it starts, carries the pointer along
    /// its line as time passes and releases where it ends, in the
    /// viewport's own place on the window.
    #[test]
    fn a_scripted_drag_presses_moves_and_releases() {
        let drag = tequila_scripting::protocol::PointerDrag { from: [0.0, 0.5], to: [1.0, 0.5], seconds: 1.0, button: "left".into(), ..Default::default() };
        let mut drag = ScriptedDrag::new(&drag);
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(200.0, 100.0));
        let start = Instant::now();
        let mut raw = egui::RawInput::default();
        assert!(drag.frame(rect, start, &mut raw));
        assert_eq!(raw.events[0], egui::Event::PointerMoved(egui::pos2(100.0, 100.0)));
        assert!(matches!(raw.events[1], egui::Event::PointerButton { pressed: true, button: egui::PointerButton::Primary, .. }));
        let mut raw = egui::RawInput::default();
        assert!(drag.frame(rect, start + Duration::from_millis(500), &mut raw));
        assert_eq!(raw.events, vec![egui::Event::PointerMoved(egui::pos2(200.0, 100.0))], "halfway along at half the time");
        let mut raw = egui::RawInput::default();
        assert!(!drag.frame(rect, start + Duration::from_secs(2), &mut raw), "over once released");
        assert_eq!(raw.events[0], egui::Event::PointerMoved(egui::pos2(300.0, 100.0)));
        assert!(matches!(raw.events[1], egui::Event::PointerButton { pressed: false, .. }));
    }

    /// The first picture after the camera has been still is the move's
    /// first; pictures of later moves of the same drag are not.
    #[test]
    fn the_first_picture_after_a_pause_starts_a_move() {
        let mut answers = Answers::default();
        let t0 = Instant::now();
        let camera = tequila_render_core::camera::Camera::default();
        answers.asked(camera, t0);
        assert!(answers.start.is_none(), "the first camera is no move");
        let mut moved = camera;
        moved.focus_distance += 1.0;
        let t1 = t0 + Duration::from_secs(1);
        answers.asked(moved, t1);
        assert_eq!(answers.start, Some(t1));
        moved.focus_distance += 1.0;
        answers.asked(moved, t1 + Duration::from_millis(10));
        assert_eq!(answers.start, Some(t1), "a move under way is no start");
        answers.shown(Some(t1), (10, 10), 1, false);
        assert!(answers.start.is_none(), "answered");
    }
}
