//! The render's size and shape, set from beside the picture.
//!
//! The size was set in the Render window alone, and someone looking at the
//! viewport asked for it near the picture: KeyShot puts it on its Image
//! tab. It is a chip in the viewport's bottom-right corner now, where the
//! render frame used to print the size, reading `1920 × 1080 · 16:9`, and
//! a click on it opens the Render window's own Image size controls over
//! the view. Both places draw [`fields`] on the document's render
//! settings, so the two cannot disagree, and a change from either is a
//! step of Undo.
//!
//! Why there:
//! - Not in the toolbar at the top right: its island is ten tools wide
//!   already, and a size there pushed the view's name off the chip at the
//!   other end in a narrow view.
//! - Not on the Image tab, as KeyShot has it: everything on that page
//!   works on the picture already made, and says so, where a new size
//!   starts the render again.
//! - In the corner the frame's bands leave empty, where the size was
//!   printed already: under the picture when the render is wider than the
//!   view, beside it when it is taller.

use crate::{theme, TequilaApp};
use eframe::egui;
use tequila_scene::render::RenderSettings;
use tequila_scene::Command;

/// Sizes people actually ask for, by the name they ask for them by.
pub(crate) const SIZES: &[(&str, usize, usize)] = &[
    ("HD 1080p", 1920, 1080),
    ("4K UHD", 3840, 2160),
    ("Square", 2048, 2048),
    ("Portrait 4:5", 1600, 2000),
    ("Story 9:16", 1080, 1920),
    ("Web banner 3:1", 2400, 800),
    ("A4 portrait, 300 DPI", 2480, 3508),
    ("A4 landscape, 300 DPI", 3508, 2480),
];

/// The least and the most a side can be set to, in pixels.
const LEAST: u32 = 16;
const MOST: u32 = 16384;

/// The chip's height: the chip at the top left's.
const CHIP_HEIGHT: f32 = crate::ISLAND - 8.0;

/// The height that keeps a ratio when the width changes.
pub(crate) fn locked_height(new_width: usize, old_width: usize, old_height: usize) -> usize {
    if old_width == 0 {
        return old_height.max(1);
    }
    ((new_width as f64 * old_height as f64 / old_width as f64).round() as usize).max(1)
}

/// `16:9`, `4:5`, `1:1` — or a decimal when the ratio has no short name.
pub(crate) fn ratio_name(width: usize, height: usize) -> String {
    fn gcd(a: usize, b: usize) -> usize {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let divisor = gcd(width, height).max(1);
    let (w, h) = (width / divisor, height / divisor);
    if w <= 32 && h <= 32 {
        format!("{w}:{h}")
    } else {
        format!("{:.2}:1", width as f64 / height.max(1) as f64)
    }
}

/// The Image size controls: a size by name, the width and the height, the
/// lock on their ratio, Swap, and the shape and megapixels they make. The
/// Render window's Image size section and the viewport's chip both draw
/// these, `id` telling the two apart. Whether the size changed.
pub(crate) fn fields(ui: &mut egui::Ui, id: &str, settings: &mut RenderSettings, lock: &mut bool) -> bool {
    let before = (settings.width, settings.height);
    ui.push_id(id, |ui| {
        let current = SIZES
            .iter()
            .find(|(_, w, h)| *w == settings.width && *h == settings.height)
            .map(|(name, _, _)| *name)
            .unwrap_or("Custom");
        egui::ComboBox::from_id_salt("size-preset")
            .selected_text(current)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for (name, w, h) in SIZES {
                    if ui.selectable_label(current == *name, format!("{name}   {w} × {h}")).clicked() {
                        settings.width = *w;
                        settings.height = *h;
                    }
                }
            });

        ui.horizontal(|ui| {
            let (old_w, old_h) = (settings.width, settings.height);
            let mut width = settings.width as u32;
            let mut height = settings.height as u32;
            // Taken when the typing is done, not at each key: with the
            // ratio locked, a width typed a digit at a time went through
            // 1, 14 and 144 on its way to 1440, each held to 16 and the
            // height worked out again from it, and the shape was lost.
            let typed = ui.add(egui::DragValue::new(&mut width).range(LEAST..=MOST).prefix("W ").suffix(" px").update_while_editing(false));
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("image-size-width", id)), typed.rect));
            if typed.changed() {
                settings.width = width as usize;
                if *lock {
                    settings.height = locked_height(settings.width, old_w, old_h).clamp(LEAST as usize, MOST as usize);
                }
            }
            ui.label("×");
            if ui.add(egui::DragValue::new(&mut height).range(LEAST..=MOST).prefix("H ").suffix(" px").update_while_editing(false)).changed() {
                settings.height = height as usize;
                if *lock {
                    settings.width = locked_height(settings.height, old_h, old_w).clamp(LEAST as usize, MOST as usize);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.toggle_value(lock, "Lock ratio")
                .on_hover_text("Keep the shape when you change the width or the height");
            if ui.button("Swap").on_hover_text("Turn landscape into portrait, and back").clicked() {
                std::mem::swap(&mut settings.width, &mut settings.height);
            }
            ui.label(
                egui::RichText::new(format!(
                    "{}  ·  {:.1} MP",
                    ratio_name(settings.width, settings.height),
                    (settings.width * settings.height) as f64 / 1e6
                ))
                .small()
                .color(theme::dim()),
            );
        });
    });
    (settings.width, settings.height) != before
}

/// How much of the chip a viewport `view` big has room for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    /// The size and the shape: `1920 × 1080 · 16:9`.
    Full,
    /// The size alone.
    Size,
    /// None: a view this small is all picture.
    Hidden,
}

fn form(view: egui::Vec2) -> Form {
    if view.x < 360.0 || view.y < 200.0 {
        Form::Hidden
    } else if view.x < 560.0 {
        Form::Size
    } else {
        Form::Full
    }
}

/// What the chip says, its size and its shape, in the form it has room for.
fn label(width: usize, height: usize, form: Form) -> (String, Option<String>) {
    let size = format!("{width} × {height}");
    let shape = (form == Form::Full).then(|| format!("· {}", ratio_name(width, height)));
    (size, shape)
}

impl TequilaApp {
    /// The size chip in the viewport's bottom-right corner, and the Image
    /// size controls it opens above itself.
    ///
    /// Its own small window rather than an egui popup: egui keeps one popup
    /// open at a time, and the list of sizes inside is one, so opening it
    /// shut the controls it was in.
    pub(crate) fn size_chip(&mut self, ui: &mut egui::Ui, area: egui::Rect) {
        let open_id = egui::Id::new("size-chip-open");
        let form = form(area.size());
        if form == Form::Hidden {
            ui.data_mut(|d| d.insert_temp(open_id, false));
            return;
        }
        let p = theme::p();
        let painter = ui.painter().clone();
        let (width, height) = {
            let settings = self.project.render_settings();
            (settings.width, settings.height)
        };
        let (size, shape) = label(width, height, form);
        let size = painter.layout_no_wrap(size, egui::FontId::proportional(12.0), p.text);
        let shape = shape.map(|shape| painter.layout_no_wrap(shape, egui::FontId::proportional(12.0), p.text_dim));
        let words = size.size().x + shape.as_ref().map_or(0.0, |g| 6.0 + g.size().x);
        // Room at the end for the arrow that says it opens.
        let chip_width = 12.0 + words + 26.0;
        let chip = egui::Rect::from_min_size(
            egui::pos2(area.right() - crate::OVERLAY_INSET - chip_width, area.bottom() - crate::OVERLAY_INSET - CHIP_HEIGHT),
            egui::vec2(chip_width, CHIP_HEIGHT),
        );
        let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
        let response = ui.interact(chip, ui.id().with("size-chip"), egui::Sense::click());
        // A script opens it as it opens a menu (`menu: "size"`).
        self.menu_buttons.retain(|(name, _)| *name != "size");
        self.menu_buttons.push(("size", chip));

        crate::icons::island(&painter, chip);
        if open || response.hovered() {
            painter.rect_stroke(chip, 11.0, egui::Stroke::new(1.0_f32, if open { p.accent } else { p.line_strong }));
        }
        let text_y = chip.center().y - size.size().y * 0.5;
        let size_width = size.size().x;
        painter.galley(egui::pos2(chip.left() + 12.0, text_y), size, egui::Color32::PLACEHOLDER);
        if let Some(shape) = shape {
            painter.galley(egui::pos2(chip.left() + 12.0 + size_width + 6.0, text_y), shape, egui::Color32::PLACEHOLDER);
        }
        // A small arrow, drawn as the chip at the top left draws its own,
        // pointing up: the controls open above.
        let tip = egui::pos2(chip.right() - 13.0, chip.center().y - 2.5);
        painter.add(egui::Shape::convex_polygon(
            vec![tip, tip + egui::vec2(3.5, 5.0), tip + egui::vec2(-3.5, 5.0)],
            p.text_dim,
            egui::Stroke::NONE,
        ));
        let response = if open {
            response
        } else {
            response.on_hover_text("The size and shape of your renders. Click to change them.")
        };
        if response.clicked() {
            open = !open;
        }

        if open {
            // A click in a list the controls opened - the sizes by name -
            // lands outside them, and is that list's, not a click away.
            let list_open = ui.memory(|m| m.any_popup_open());
            let mut settings = self.project.render_settings().clone();
            let mut changed = false;
            let mut frame = self.show_render_frame;
            let shown = egui::Area::new(egui::Id::new("size-chip-controls"))
                .order(egui::Order::Foreground)
                .fixed_pos(chip.right_top() - egui::vec2(0.0, 6.0))
                .pivot(egui::Align2::RIGHT_BOTTOM)
                .constrain(true)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(262.0);
                        theme::section(ui, "Image size");
                        changed = self.render_window.size_fields(ui, &mut settings);
                        ui.add_space(2.0);
                        ui.add(crate::field::Checkbox::new(&mut frame, "Show the frame in the view")).on_hover_text(
                            "The view at the render's shape, the rest of it left empty, so what you see is what renders. The same as View › Render frame.",
                        );
                        theme::note(ui, "The size renders are made at. The Render window has the same settings, and a print size.");
                    });
                });
            self.show_render_frame = frame;
            if changed {
                let _ = self.project.apply(Command::SetRenderSettings { settings: Box::new(settings) });
            }
            // Pressed anywhere else - on the view, to turn it - closes them,
            // as a menu closes.
            let away = ui.input(|i| {
                i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|at| !shown.response.rect.contains(at) && !chip.contains(at))
            });
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if !list_open && (away || escape) {
                open = false;
            }
        }
        ui.data_mut(|d| d.insert_temp(open_id, open));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locked_ratio_follows_the_width() {
        assert_eq!(locked_height(3840, 1920, 1080), 2160);
        assert_eq!(locked_height(1000, 1600, 2000), 1250);
        // A degenerate old size does not divide by zero.
        assert_eq!(locked_height(500, 0, 300), 300);
    }

    #[test]
    fn ratios_have_their_short_names() {
        assert_eq!(ratio_name(1920, 1080), "16:9");
        assert_eq!(ratio_name(1600, 2000), "4:5");
        assert_eq!(ratio_name(2048, 2048), "1:1");
        assert_eq!(ratio_name(2480, 3508), "0.71:1");
        assert_eq!(ratio_name(1436, 1077), "4:3");
    }

    /// The chip says the size and the shape, then the size alone in a
    /// narrower view, and goes in a view too small for it.
    #[test]
    fn the_chip_says_the_size_and_shape_in_the_room_it_has() {
        assert_eq!(form(egui::vec2(1100.0, 700.0)), Form::Full);
        assert_eq!(label(1436, 1077, Form::Full), ("1436 × 1077".to_string(), Some("· 4:3".to_string())));
        assert_eq!(form(egui::vec2(480.0, 700.0)), Form::Size);
        assert_eq!(label(1920, 1080, Form::Size), ("1920 × 1080".to_string(), None));
        assert_eq!(form(egui::vec2(340.0, 700.0)), Form::Hidden);
        assert_eq!(form(egui::vec2(900.0, 160.0)), Form::Hidden);
    }

    /// Frames of the viewport alone, `size` big, at a time that moves on.
    struct Viewport {
        ctx: egui::Context,
        time: f64,
        size: egui::Vec2,
    }

    impl Viewport {
        fn new(size: egui::Vec2) -> Viewport {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            crate::theme::style_for_tests(&ctx);
            Viewport { ctx, time: 0.0, size }
        }

        fn frame(&mut self, app: &mut TequilaApp, events: Vec<egui::Event>) {
            self.time += 0.1;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(1.0);
            let _ = self.ctx.run(input, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| app.viewport(ui));
            });
        }

        fn click(&mut self, app: &mut TequilaApp, at: egui::Pos2) {
            let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            self.frame(app, vec![egui::Event::PointerMoved(at), press(true)]);
            self.frame(app, vec![press(false)]);
        }

        fn is_open(&self) -> bool {
            self.ctx.data(|d| d.get_temp::<bool>(egui::Id::new("size-chip-open"))).unwrap_or(false)
        }
    }

    fn size_of(app: &TequilaApp) -> (usize, usize) {
        let settings = app.project.render_settings();
        (settings.width, settings.height)
    }

    /// The chip opens the Render window's own controls, on the render's
    /// own size: a width typed there with the Render window's lock on
    /// changes the document's size, height and all, which the frame in
    /// the view follows; one Undo puts it back; a click away closes them.
    #[test]
    fn the_chip_and_the_render_window_edit_the_same_size() {
        let mut app = TequilaApp::for_tests();
        let settings = RenderSettings { width: 1920, height: 1080, ..app.project.render_settings().clone() };
        app.project.apply(Command::SetRenderSettings { settings: Box::new(settings) }).unwrap();
        app.project.end_interaction();
        // The lock is the Render window's: the two share it.
        app.render_window.lock_ratio = true;
        let mut view = Viewport::new(egui::vec2(900.0, 600.0));
        view.frame(&mut app, Vec::new());
        assert!((app.viewport_rect.aspect_ratio() - 16.0 / 9.0).abs() < 0.01, "the frame is not 16:9: {:?}", app.viewport_rect);
        let chip = app.menu_buttons.iter().find(|(name, _)| *name == "size").map(|(_, rect)| *rect).expect("no size chip");
        assert!(chip.max.x <= 900.0 - crate::OVERLAY_INSET + 0.5 && chip.max.y <= 600.0 - crate::OVERLAY_INSET + 0.5, "the chip is not in the corner: {chip:?}");

        view.click(&mut app, chip.center());
        view.frame(&mut app, Vec::new());
        assert!(view.is_open(), "a click on the chip did not open the controls");

        // A width typed into the controls, as a person would: a click on
        // the field, everything in it chosen, the new width, Enter.
        let field = view.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("image-size-width", "viewport-size")))).expect("no width field in the controls");
        view.click(&mut app, field.center());
        view.frame(&mut app, vec![egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }]);
        view.frame(&mut app, vec![egui::Event::Text("1440".into())]);
        assert_eq!(size_of(&app), (1920, 1080), "the width was taken before the typing was done");
        view.frame(&mut app, vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        view.frame(&mut app, Vec::new());
        assert_eq!(size_of(&app), (1440, 810), "the typed width, and the height that keeps 16:9");
        assert!(view.is_open(), "typing closed the controls");

        // One step of Undo, as from the Render window.
        app.project.undo();
        assert_eq!(size_of(&app), (1920, 1080));

        // The Render window's change, read by the chip and the frame.
        let settings = RenderSettings { width: 1080, height: 1350, ..app.project.render_settings().clone() };
        app.project.apply(Command::SetRenderSettings { settings: Box::new(settings) }).unwrap();
        view.frame(&mut app, Vec::new());
        assert!((app.viewport_rect.aspect_ratio() - 0.8).abs() < 0.01, "the frame did not follow: {:?}", app.viewport_rect);

        // A click on the view closes the controls.
        view.click(&mut app, egui::pos2(300.0, 200.0));
        assert!(!view.is_open(), "a click away left the controls open");
    }

    /// In a viewport too small for it the chip is not drawn, and a script
    /// has nothing to open.
    #[test]
    fn a_small_view_has_no_chip() {
        let mut app = TequilaApp::for_tests();
        let mut view = Viewport::new(egui::vec2(320.0, 240.0));
        view.frame(&mut app, Vec::new());
        assert!(app.menu_buttons.iter().all(|(name, _)| *name != "size"));
    }
}
