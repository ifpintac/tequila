//! Tequila's own windows — the Render window, the material graph, the
//! Shortcuts and the AI connections — as windows of the operating system.
//!
//! Each opens as a window of its own rather than a panel drawn inside the
//! main one, so it can be moved off the main window, onto a second display,
//! and sized past it: a node graph or a finished render wants the room, and
//! a window that cannot leave the one it was opened from covers the very
//! model it is about. Where a second window cannot be opened — a test's
//! context, a platform eframe embeds on — the same contents show inside the
//! main window, as they used to.
//!
//! They are egui's *immediate* viewports: drawn during the main window's
//! frame, with the application's state at hand, so nothing had to be made
//! shareable between threads for them. Each window:
//! - reopens where it was left, at the size it was left;
//! - has its title bar follow light and dark with the interface;
//! - passes undo, redo and save back to the main window's keys, since those
//!   mean the same whichever window has the keyboard;
//! - can be captured by the scripting channel's `screenshot`.

use eframe::egui;
use std::sync::{Arc, Mutex};

/// One of the windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Render,
    Graph,
    Shortcuts,
    Connect,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Render, Kind::Graph, Kind::Shortcuts, Kind::Connect];

    /// Its name on the scripting channel.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Render => "render",
            Kind::Graph => "graph",
            Kind::Shortcuts => "shortcuts",
            Kind::Connect => "connect",
        }
    }

    pub fn from_key(key: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.key().eq_ignore_ascii_case(key))
    }

    fn viewport(self) -> egui::ViewportId {
        egui::ViewportId::from_hash_of(("tequila-window", self.key()))
    }

    fn id(self) -> egui::Id {
        egui::Id::new(("tequila-window", self.key()))
    }
}

/// How a window opens.
pub struct Spec {
    pub kind: Kind,
    pub title: String,
    /// Its size the first time it opens, in points.
    pub size: egui::Vec2,
    pub min_size: egui::Vec2,
    pub resizable: bool,
    /// Space between the window's edges and its contents.
    pub margin: f32,
}

/// What showing a window reported.
pub struct Shown<R> {
    /// What its contents returned, if they were drawn.
    pub inner: Option<R>,
    /// Whether the pointer is over it inside the main window: only ever
    /// true when it is embedded, where its keys and the main window's are
    /// one input. A window of its own has its own keys.
    pub hovered_inside: bool,
}

/// Where a window is, kept between frames.
#[derive(Clone, Copy, Default)]
struct Placement {
    /// Where it opened and at what size. Held unchanged while it is open:
    /// egui moves and resizes a window whose description changes, so
    /// describing it from where it is now would fight the person moving it.
    opened: Option<(Option<egui::Pos2>, egui::Vec2)>,
    /// Where it was and how large, as last seen: where it reopens.
    last: Option<(egui::Pos2, egui::Vec2)>,
    /// The main window's pass it was last shown on, to tell a window that
    /// was closed — by any route — from one still open.
    seen: u64,
    /// The mode its title bar was last set to.
    dark: Option<bool>,
    /// Frames drawn since it opened. A new window is created hidden and
    /// shown on a later frame: the desktop creates it where it likes at a
    /// size of its own, and moves and sizes it as asked a moment later,
    /// which shown would be a window jumping into place.
    frames: u32,
}

/// Frames a new window stays hidden while it is put in place.
const HIDDEN_FRAMES: u32 = 2;

/// Show a window, `open` while it is, its contents drawn by `add`.
/// `embedded` finishes the in-window fallback's description.
pub fn show<R>(
    ctx: &egui::Context,
    spec: Spec,
    open: &mut bool,
    embedded: impl FnOnce(egui::Window<'_>) -> egui::Window<'_>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Shown<R> {
    let id = spec.kind.id();
    let pass = ctx.cumulative_pass_nr();
    let mut placement: Placement = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    if placement.seen + 1 < pass {
        // Not shown on the last pass: it was closed, and this is it opening
        // again — where it was left.
        placement.opened = None;
        placement.dark = None;
    }
    if placement.opened.is_none() {
        placement.frames = 0;
    }
    let (at, size) = *placement.opened.get_or_insert_with(|| {
        let size = placement.last.map_or(spec.size, |(_, size)| size);
        (placement.last.map(|(at, _)| at).or_else(|| centred(ctx, size)), size)
    });
    let revealed = placement.frames >= HIDDEN_FRAMES;
    if !revealed {
        ctx.request_repaint();
    }
    let mut builder = egui::ViewportBuilder::default()
        .with_title(spec.title.clone())
        .with_inner_size(size)
        .with_min_inner_size(spec.min_size)
        .with_resizable(spec.resizable)
        .with_visible(revealed);
    if let Some(at) = at {
        builder = builder.with_position(at);
    }

    let mut add = Some(add);
    let mut embedded = Some(embedded);
    let mut hovered_inside = false;
    let mut last = None;
    let mut dark = placement.dark;
    let inner = ctx.show_viewport_immediate(spec.kind.viewport(), builder, |ctx, class| {
        if class == egui::ViewportClass::Embedded {
            let window = egui::Window::new(spec.title.as_str()).id(id).open(&mut *open).collapsible(false);
            let window = match embedded.take() {
                Some(finish) => finish(window),
                None => window,
            };
            let shown = window.show(ctx, |ui| add.take().map(|add| add(ui)));
            hovered_inside = shown.as_ref().is_some_and(|w| w.response.contains_pointer());
            return shown.and_then(|w| w.inner).flatten();
        }
        // Its own title bar, in the interface's mode rather than the
        // desktop's: a dark window under a white title bar looks broken.
        let p = crate::theme::p();
        if dark != Some(p.dark) {
            ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(if p.dark {
                egui::SystemTheme::Dark
            } else {
                egui::SystemTheme::Light
            }));
            dark = Some(p.dark);
        }
        let inner = egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(p.panel).inner_margin(spec.margin))
            .show(ctx, |ui| add.take().map(|add| add(ui)))
            .inner;
        shared_keys(ctx);
        capture(ctx, spec.kind);
        // Where it is, once it is shown: while hidden it is still on its
        // way there.
        if revealed {
            last = ctx.input(|i| {
                let viewport = i.viewport();
                viewport.outer_rect.zip(viewport.inner_rect).map(|(outer, inner)| (outer.min, inner.size()))
            });
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            *open = false;
        }
        inner
    });

    placement.seen = pass;
    placement.dark = dark;
    placement.frames = placement.frames.saturating_add(1);
    if let Some(last) = last {
        placement.last = Some(last);
    }
    if !*open {
        placement.opened = None;
        placement.dark = None;
    }
    ctx.data_mut(|d| d.insert_temp(id, placement));
    Shown { inner, hovered_inside }
}

/// Where a window of `size` sits centred on the main window, if the main
/// window says where it is.
fn centred(ctx: &egui::Context, size: egui::Vec2) -> Option<egui::Pos2> {
    let main = ctx.input(|i| i.viewport().outer_rect)?;
    // Its title bar is above the inner size asked for: lift it by about
    // half of one, so the whole window is centred, not just its inside.
    Some(main.center() - size * 0.5 - egui::vec2(0.0, 16.0))
}

// ---- keys ------------------------------------------------------------------

/// A key that means the same in every window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shared {
    Undo,
    Redo,
    Save,
}

fn keys_id() -> egui::Id {
    egui::Id::new("tequila-window-keys")
}

/// Undo, redo and save pressed in one of these windows, for the main
/// window to carry out — not while a text field has the keyboard, which
/// has its own undo.
fn shared_keys(ctx: &egui::Context) {
    if ctx.memory(|m| m.focused().is_some()) {
        return;
    }
    // Each press with the modifiers it was pressed with, rather than the
    // frame's: two in one frame can differ.
    let pressed: Vec<Shared> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } if modifiers.command => match key {
                    egui::Key::Z if modifiers.shift => Some(Shared::Redo),
                    egui::Key::Z => Some(Shared::Undo),
                    egui::Key::Y => Some(Shared::Redo),
                    egui::Key::S => Some(Shared::Save),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    });
    if !pressed.is_empty() {
        ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<Shared>>(keys_id()).extend(pressed));
    }
}

/// The shared keys pressed in the windows since the last call.
pub fn take_keys(ctx: &egui::Context) -> Vec<Shared> {
    ctx.data_mut(|d| d.remove_temp::<Vec<Shared>>(keys_id())).unwrap_or_default()
}

// ---- screenshots -----------------------------------------------------------

/// A screenshot asked of one of the windows. egui 0.29 hands back
/// screenshots of the main window only, so a window's own frame is read
/// back as it is drawn, by a callback painted over everything else in it.
struct Capture {
    kind: Kind,
    path: std::path::PathBuf,
    image: Option<Arc<Mutex<Option<egui::ColorImage>>>>,
    done: Option<Result<(), String>>,
}

static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);

/// Ask for a PNG of window `kind` at `path`, taken on its next frames.
pub fn request_capture(kind: Kind, path: std::path::PathBuf) {
    *CAPTURE.lock().unwrap() = Some(Capture { kind, path, image: None, done: None });
}

/// Whether the capture asked for has been written, and how it went.
pub fn capture_done() -> Option<Result<(), String>> {
    let mut capture = CAPTURE.lock().unwrap();
    if capture.as_ref().is_some_and(|c| c.done.is_some()) {
        return capture.take().and_then(|c| c.done);
    }
    None
}

/// Give up on the capture asked for: its window closed first.
pub fn cancel_capture() {
    *CAPTURE.lock().unwrap() = None;
}

fn capture(ctx: &egui::Context, kind: Kind) {
    let mut guard = CAPTURE.lock().unwrap();
    let Some(capture) = guard.as_mut().filter(|c| c.kind == kind && c.done.is_none()) else {
        return;
    };
    match &capture.image {
        None => {
            let slot = Arc::new(Mutex::new(None));
            let filled = slot.clone();
            let screen = ctx.screen_rect();
            ctx.layer_painter(egui::LayerId::new(egui::Order::Debug, egui::Id::new("tequila-window-capture"))).add(
                egui::PaintCallback {
                    rect: screen,
                    callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, painter| {
                        *filled.lock().unwrap() = Some(painter.read_screen_rgba(info.screen_size_px));
                    })),
                },
            );
            capture.image = Some(slot);
            ctx.request_repaint();
        }
        Some(slot) => {
            let taken = slot.lock().unwrap().take();
            match taken {
                Some(mut image) => {
                    // Opaque, as the main window's screenshots are: the
                    // window is cleared to transparent before it is drawn.
                    for pixel in &mut image.pixels {
                        *pixel = egui::Color32::from_rgb(pixel.r(), pixel.g(), pixel.b());
                    }
                    capture.done = Some(crate::channel::write_png(&capture.path, &image));
                }
                None => ctx.request_repaint(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without a platform to open windows on — a test's context — a window
    /// is drawn inside the main one, its contents and its close button
    /// working as before.
    #[test]
    fn without_a_platform_a_window_is_drawn_inside() {
        let ctx = egui::Context::default();
        let mut open = true;
        let mut drew = false;
        let mut hovered = false;
        let _ = ctx.run(Default::default(), |ctx| {
            let shown = show(
                ctx,
                Spec {
                    kind: Kind::Shortcuts,
                    title: "Shortcuts".into(),
                    size: egui::vec2(400.0, 300.0),
                    min_size: egui::vec2(200.0, 100.0),
                    resizable: true,
                    margin: 10.0,
                },
                &mut open,
                |window| window.default_pos([0.0, 0.0]),
                |ui| {
                    ui.label("contents");
                    drew = true;
                    42
                },
            );
            assert_eq!(shown.inner, Some(42));
            hovered = shown.hovered_inside;
        });
        assert!(drew, "the contents were drawn");
        assert!(open, "and the window stays open");
        assert!(!hovered, "with no pointer over it");
    }

    /// Undo, redo and save pressed in a window are passed on to the main
    /// window's keys, once each; other keys, and keys typed into a text
    /// field, are not.
    #[test]
    fn undo_redo_and_save_are_passed_on() {
        let ctx = egui::Context::default();
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let shift_command = egui::Modifiers { shift: true, ..egui::Modifiers::COMMAND };
        let input = egui::RawInput {
            events: vec![
                key(egui::Key::Z, egui::Modifiers::COMMAND),
                key(egui::Key::Z, shift_command),
                key(egui::Key::S, egui::Modifiers::COMMAND),
                key(egui::Key::G, egui::Modifiers::NONE),
            ],
            ..Default::default()
        };
        let _ = ctx.run(input, shared_keys);
        assert_eq!(take_keys(&ctx), vec![Shared::Undo, Shared::Redo, Shared::Save]);
        assert!(take_keys(&ctx).is_empty(), "each is passed on once");

        // With a text field focused, Ctrl+Z is the field's own undo.
        let input = egui::RawInput { events: vec![key(egui::Key::Z, egui::Modifiers::COMMAND)], ..Default::default() };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut text = String::new();
                ui.add(egui::TextEdit::singleline(&mut text)).request_focus();
            });
            shared_keys(ctx);
        });
        assert!(take_keys(&ctx).is_empty(), "not while typing");
    }

    /// A window's name on the scripting channel finds it, in any case.
    #[test]
    fn windows_are_found_by_name() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_key(kind.key()), Some(kind));
            assert_eq!(Kind::from_key(&kind.key().to_uppercase()), Some(kind));
        }
        assert_eq!(Kind::from_key("viewport"), None);
    }
}
