//! Pick focus: the Camera tab's crosshair, then a click on the model sets
//! the focus distance to the point clicked (`TEQUILA_SPEC.md` §1.7,
//! "click-to-focus").
//!
//! Armed, the viewport's pointer is a crosshair and a line at the foot of the
//! view says what a click does. A click on a part focuses there and ends the
//! picking; a click on the background or the floor does nothing and leaves it
//! on, since empty space is what the selection's click takes it to be too
//! (`pick_at`). Picking takes the click from the selection and the gizmo, as
//! placing a label does, but not the drag: the view can still be turned while
//! it is armed, as the measuring tool's can.
//!
//! What the focus distance of a point *is* - its depth along the view axis,
//! which is the plane both renderers hold sharp - is
//! [`tequila_scene::focus`]'s, shared with a script's `look` with `focus`.

use crate::gizmo_ui;
use crate::orbit::OrbitCamera;
use crate::orchestrator::Orchestrator;
use crate::project::Project;
use crate::TequilaApp;
use eframe::egui;
use std::time::{Duration, Instant};
use tequila_core::math::Vec3;
use tequila_render_core::camera::Camera;
use tequila_scene::focus::{can_focus, focus_on, CannotFocus, Focused};
use tequila_scene::Command;

/// How long the mark where the focus landed stays, fading as it goes.
const MARK_FOR: Duration = Duration::from_millis(1600);

#[derive(Default)]
pub struct FocusPick {
    armed: bool,
    /// Where the last pick landed, and when, for the mark that shows it for
    /// a moment.
    landed: Option<(Vec3, Instant)>,
}

impl FocusPick {
    pub fn armed(&self) -> bool {
        self.armed
    }

    pub fn set(&mut self, on: bool) {
        self.armed = on;
    }

    /// Take the click, if picking. Returns whether it took this frame's
    /// click or Escape, which the selection and the gizmo must then leave
    /// alone. A drag is left to turn the view.
    #[allow(clippy::too_many_arguments)]
    pub fn interact(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        orbit: &mut OrbitCamera,
        project: &mut Project,
        orchestrator: &Orchestrator,
        response: &egui::Response,
        toasts: &mut crate::toast::Toasts,
        blur_shown: bool,
    ) -> bool {
        if !self.armed {
            return false;
        }
        // The lens changed to one with nothing to focus while picking.
        if !can_focus(&orbit.camera.projection) {
            self.armed = false;
            project.status = CannotFocus::NoBlur.message().into();
            return false;
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.armed = false;
            project.status = "Focus left as it was".into();
            return true;
        }
        if !response.clicked() {
            return false;
        }
        let Some(at) = response.interact_pointer_pos() else {
            return true;
        };
        // A part of the model, and nothing else: the floor and the
        // background are empty space, as they are to a selecting click.
        let point = gizmo_ui::ray_through(&orbit.camera, rect, at)
            .and_then(|ray| orchestrator.pick(&ray))
            .filter(|hit| crate::clicked_part(project.mapping(), hit).is_some())
            .map(|hit| hit.position);
        if let Some(point) = point {
            match self.focus_at(point, orbit, project) {
                Ok(focused) => toasts.info(said(&focused, project, blur_shown)),
                Err(why) => toasts.warn(why),
            }
        }
        true
    }

    /// Focus on `point`, picked: as [`focus`], and the picking ends with a
    /// mark where it landed. A point that cannot be focused on leaves the
    /// picking on, to try another.
    pub(crate) fn focus_at(&mut self, point: Vec3, orbit: &mut OrbitCamera, project: &mut Project) -> Result<Focused, String> {
        let focused = focus(orbit, project, point)?;
        self.armed = false;
        self.landed = Some((point, Instant::now()));
        Ok(focused)
    }

    /// The line saying what a click does, while picking, and for a moment
    /// after one the mark where the focus landed.
    ///
    /// Painted after the viewport image, like every other overlay: egui draws
    /// in call order within a layer and the image covers the viewport.
    pub fn draw(&self, ui: &egui::Ui, rect: egui::Rect, camera: &Camera) {
        let painter = ui.painter_at(rect);
        if self.armed {
            crate::label_gizmo::banner(&painter, rect, &hint(camera));
        }
        let Some((point, when)) = self.landed else {
            return;
        };
        let age = when.elapsed();
        if age >= MARK_FOR {
            return;
        }
        // Still for most of its time, then fading.
        let fade = (1.0 - age.as_secs_f32() / MARK_FOR.as_secs_f32()).min(0.35) / 0.35;
        if let Some(at) = gizmo_ui::project_point(camera, rect, point) {
            mark(&painter, at, fade);
        }
        ui.ctx().request_repaint();
    }
}

/// What the line at the foot of the view says while picking.
fn hint(camera: &Camera) -> String {
    let blurs = camera.f_number.is_finite() && camera.f_number > 0.0;
    if blurs {
        "Click the point to focus on (Esc cancels)".into()
    } else {
        "Click the point to focus on (Esc cancels) · focus blur is off, and this turns it on".into()
    }
}

/// What the person is told once the focus is set.
fn said(focused: &Focused, project: &Project, blur_shown: bool) -> String {
    let mut said = format!(
        "In focus {} from the camera.",
        tequila_scene::measure::format_length(focused.distance, &project.document().units)
    );
    if focused.blur_turned_on {
        said.push_str(" Focus blur was off, so it is on now.");
    }
    if focused.keep_focus_turned_off {
        said.push_str(" Keep focus is off now, so turning the view leaves the focus where it is.");
    }
    if !blur_shown {
        said.push_str(" The blur shows in the Rendered view and in renders.");
    }
    said
}

/// A ring with four ticks where the focus landed, dark under light so it
/// reads over a white part and a black one.
fn mark(painter: &egui::Painter, at: egui::Pos2, fade: f32) {
    let light = egui::Color32::from_rgb(120, 220, 255).gamma_multiply(fade);
    let dark = egui::Color32::from_black_alpha(160).gamma_multiply(fade);
    let ticks = |stroke: egui::Stroke| {
        for direction in [egui::vec2(1.0, 0.0), egui::vec2(-1.0, 0.0), egui::vec2(0.0, 1.0), egui::vec2(0.0, -1.0)] {
            painter.line_segment([at + direction * 6.0, at + direction * 14.0], stroke);
        }
    };
    painter.circle_stroke(at, 10.0, egui::Stroke::new(3.5_f32, dark));
    ticks(egui::Stroke::new(3.5_f32, dark));
    painter.circle_stroke(at, 10.0, egui::Stroke::new(1.6_f32, light));
    ticks(egui::Stroke::new(1.6_f32, light));
    painter.circle_filled(at, 2.0, light);
}

/// Focus the view's camera on `point`, as one step to undo: the focus
/// distance its depth, the blur turned on if it was off, Keep focus turned
/// off (`tequila_scene::focus::focus_on`). Nothing changes when it cannot.
pub(crate) fn focus(orbit: &mut OrbitCamera, project: &mut Project, point: Vec3) -> Result<Focused, String> {
    let mut camera = orbit.camera;
    let focused = focus_on(&mut camera, point).map_err(|why| why.message().to_string())?;
    let before = orbit.camera;
    orbit.camera = camera;
    let index = project.document().cameras.active_index();
    let name = project.camera().name.clone();
    // A step of its own: camera commands merge as a drag's frames do, and a
    // pick straight after a turn of the wheel would otherwise be undone with
    // the zoom.
    project.end_interaction();
    let applied = project.apply(Command::SetCamera { index, camera: Box::new(orbit.to_saved(name)) });
    project.end_interaction();
    match applied {
        Ok(()) => Ok(focused),
        Err(error) => {
            orbit.camera = before;
            Err(error.to_string())
        }
    }
}

/// The Camera tab's Pick focus button: on, or off again. Turning it on puts
/// away the other tools that take the viewport's click - placing a label or
/// a texture, aiming a light, measuring - so one click does one thing.
pub(crate) fn toggle(app: &mut TequilaApp) {
    set(app, !app.focus_pick.armed());
}

/// Picking on or off, as [`toggle`] does.
pub(crate) fn set(app: &mut TequilaApp, on: bool) {
    if on {
        app.label_gizmo.disarm();
        app.mapping_gizmo.disarm();
        app.pin_gizmo.disarm();
        if app.measure.active() {
            app.measure.toggle();
        }
    }
    app.focus_pick.set(on && can_focus(&app.camera.camera.projection));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scene::focus::{focus_depth, BLUR_F_NUMBER};

    /// One frame of the viewport, `size` points, at a pixel a point, with
    /// these events.
    fn frame(app: &mut TequilaApp, ctx: &egui::Context, size: egui::Vec2, events: Vec<egui::Event>) {
        let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(1.0);
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| app.viewport(ui));
        });
    }

    /// A click as a mouse makes one: pressed in one frame, let go in the
    /// next, in the same place.
    fn click(app: &mut TequilaApp, ctx: &egui::Context, size: egui::Vec2, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(app, ctx, size, vec![egui::Event::PointerMoved(at), button(true)]);
        frame(app, ctx, size, vec![button(false)]);
        frame(app, ctx, size, Vec::new());
    }

    /// A demo scene in the Rendered view, its pickable geometry built, with
    /// a place on a part (the nearest point there, and the part) and a place
    /// on nothing a part, found by casting through the view.
    fn ready() -> (TequilaApp, egui::Context, egui::Vec2, (egui::Pos2, Vec3), egui::Pos2) {
        let mut app = TequilaApp::for_tests();
        app.show_render_frame = false;
        let settings = tequila_scene::render::RenderSettings {
            engine: tequila_scene::render::RenderEngine::Tequila,
            ..app.project.render_settings().clone()
        };
        app.project.apply(Command::SetRenderSettings { settings: Box::new(settings) }).unwrap();
        app.set_view_mode(crate::view_mode::ViewMode::Rendered);
        app.camera.set_standard_view(tequila_scene::orbit::StandardView::Isometric);
        app.frame(true);
        app.sync_viewport_camera();
        app.project.end_interaction();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let size = egui::vec2(240.0, 180.0);
        frame(&mut app, &ctx, size, Vec::new());
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.orchestrator.picker().is_none() {
            assert!(Instant::now() < deadline, "the viewport built nothing to pick");
            std::thread::sleep(Duration::from_millis(5));
        }
        let picker = app.orchestrator.picker().unwrap();
        let rect = app.viewport_rect;
        let (mut part, mut empty) = (None, None);
        for y in (2..rect.height() as i32).step_by(4) {
            for x in (2..rect.width() as i32).step_by(4) {
                let at = rect.min + egui::vec2(x as f32, y as f32);
                match gizmo_ui::ray_through(&app.camera.camera, rect, at).and_then(|ray| picker.pick(&ray)) {
                    Some(hit) if crate::clicked_part(app.project.mapping(), &hit).is_some() => {
                        part.get_or_insert((at, hit.position));
                    }
                    _ => {
                        empty.get_or_insert(at);
                    }
                }
            }
        }
        (app, ctx, size, part.expect("a part in view"), empty.expect("something in view that is not a part"))
    }

    /// Armed, a click on a part sets the focus to that point's depth along
    /// the view axis, turns the blur on, selects nothing and ends the
    /// picking; one undo puts the camera back as it was.
    #[test]
    fn a_click_on_a_part_focuses_there_in_one_step_and_selects_nothing() {
        let (mut app, ctx, size, (on_part, point), _) = ready();
        assert!(!app.camera.camera.f_number.is_finite(), "the demo camera starts sharp all through");
        app.project.selection.clear();
        let before = app.project.camera().clone();
        toggle(&mut app);
        assert!(app.focus_pick.armed());

        click(&mut app, &ctx, size, on_part);
        assert!(!app.focus_pick.armed(), "picking goes off once the focus is set");
        let camera = app.project.camera().camera;
        let depth = focus_depth(&camera, point).expect("in front");
        assert!((camera.focus_distance - depth).abs() < 1e-4 * depth.max(1.0), "focused at {}, the point is {depth} deep", camera.focus_distance);
        assert_eq!(camera.f_number, BLUR_F_NUMBER, "the blur was turned on");
        assert_eq!(app.camera.camera, camera, "the view's camera is the document's");
        assert!(app.project.selection.active().is_none(), "the click selected the part");
        assert!(app.landed_mark_shown(), "no mark where the focus landed");

        app.undo();
        assert_eq!(*app.project.camera(), before, "one undo did not put the camera back");
        assert_eq!(app.camera.camera, before.camera);
    }

    /// Armed, a click on the floor or the background changes nothing and
    /// leaves the picking on; Escape then turns it off, still changing
    /// nothing.
    #[test]
    fn a_click_on_empty_space_changes_nothing_and_keeps_picking() {
        let (mut app, ctx, size, _, empty) = ready();
        let before = app.project.camera().clone();
        let step = app.project.undo_step();
        toggle(&mut app);
        click(&mut app, &ctx, size, empty);
        assert!(app.focus_pick.armed(), "a click on nothing turned picking off");
        assert_eq!(*app.project.camera(), before, "a click on nothing changed the camera");
        assert_eq!(app.project.undo_step(), step, "a click on nothing made a step to undo");

        let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(&mut app, &ctx, size, vec![escape]);
        assert!(!app.focus_pick.armed(), "Escape did not end the picking");
        assert_eq!(*app.project.camera(), before);
    }

    /// A pick right after a turn of the wheel is its own step to undo, not
    /// folded into the zoom as two camera edits in a row would be.
    #[test]
    fn a_pick_after_a_zoom_is_its_own_undo_step() {
        let mut app = TequilaApp::for_tests();
        app.sync_viewport_camera();
        app.project.end_interaction();
        app.camera.zoom(1.0);
        app.sync_viewport_camera();
        let zoomed = app.project.camera().clone();
        let target = app.camera.target;
        focus(&mut app.camera, &mut app.project, target).expect("focuses");
        app.undo();
        assert_eq!(*app.project.camera(), zoomed, "the undo took the zoom with the focus");
    }

    /// The tools that take the viewport's click are put away when picking
    /// starts, and picking is not armed on a lens with nothing to focus.
    #[test]
    fn picking_puts_the_other_click_tools_away() {
        let mut app = TequilaApp::for_tests();
        app.measure.toggle();
        set(&mut app, true);
        assert!(app.focus_pick.armed() && !app.measure.active());
        toggle(&mut app);
        assert!(!app.focus_pick.armed(), "the button turns it off again");

        app.camera.camera.projection = tequila_render_core::camera::Projection::Orthographic { scale: 1.0 };
        set(&mut app, true);
        assert!(!app.focus_pick.armed(), "an orthographic camera has no focus to pick");
    }

    /// What the line and the notice say: the blur turned on, Keep focus
    /// turned off, and where the blur can be seen.
    #[test]
    fn the_hint_and_the_notice_say_what_changed() {
        let sharp = Camera::default();
        assert!(hint(&sharp).contains("turns it on"), "{}", hint(&sharp));
        let blurred = Camera { f_number: 4.0, ..Camera::default() };
        assert_eq!(hint(&blurred), "Click the point to focus on (Esc cancels)");

        let app = TequilaApp::for_tests();
        let all = Focused { distance: 0.5, blur_turned_on: true, keep_focus_turned_off: true };
        let text = said(&all, &app.project, false);
        assert!(text.contains("on now") && text.contains("Keep focus is off") && text.contains("Rendered view"), "{text}");
        let plain = said(&Focused { blur_turned_on: false, keep_focus_turned_off: false, ..all }, &app.project, true);
        assert!(plain.starts_with("In focus") && !plain.contains("Keep focus") && !plain.contains("Rendered"), "{plain}");
    }
}

#[cfg(test)]
impl TequilaApp {
    /// Whether the mark where the focus last landed is still showing.
    fn landed_mark_shown(&self) -> bool {
        self.focus_pick.landed.is_some_and(|(_, when)| when.elapsed() < MARK_FOR)
    }
}
