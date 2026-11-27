//! Looking at a render in the Render window: zoom and pan over its stage,
//! and the toolbar that floats over the picture - zoom out and in, Fit and
//! 100%, and while a render runs, Pause and Stop (`docs/04-UX-SPEC.md` §11).
//!
//! # Zoom
//!
//! The zoom is screen pixels to a pixel of the render, so 100% shows each of
//! the render's pixels as one of the screen's whatever the display's scaling:
//! on a laptop set to 150%, a 1920-pixel render at 100% is 1280 points
//! across. Fit, where every picture starts, follows the stage as the window
//! is resized; any other zoom keeps its scale, about the point of the
//! picture at the middle of the stage. The view is the window's, not the
//! picture's, so two renders of a size are looked at in the same place.
//!
//! # Filtering
//!
//! Past 100% each pixel of the render is drawn as a square with hard edges
//! (nearest filtering). Zooming in is for judging pixels - the noise left, a
//! firefly, an edge's antialiasing - and a smooth filter blurs exactly what
//! is being looked for: it makes a noisy render look cleaner than it is. At
//! 100% the picture is put on whole pixels of the screen, so each of its
//! pixels lands on one and the filter does not matter. Below 100%, and at
//! Fit, the picture is smoothed as it always was: magnified a little to fit
//! a large stage, a small render's pixels would otherwise come out unevenly
//! doubled.

use crate::icons::{self, Icon, Tone};
use crate::theme;
use eframe::egui;
use egui::{Pos2, Rect, Vec2};
use std::sync::Arc;

/// The closest the stage goes: one pixel of the render 32 screen pixels
/// across.
pub const MAX_ZOOM: f32 = 32.0;

/// The zooms the buttons and keys step through, as an image viewer's.
const STEPS: [f32; 18] = [
    1.0 / 16.0,
    1.0 / 8.0,
    1.0 / 6.0,
    1.0 / 4.0,
    1.0 / 3.0,
    1.0 / 2.0,
    2.0 / 3.0,
    1.0,
    1.5,
    2.0,
    3.0,
    4.0,
    6.0,
    8.0,
    12.0,
    16.0,
    24.0,
    MAX_ZOOM,
];

/// How far one notch of a mouse wheel zooms.
const WHEEL_STEP: f32 = 1.2;

/// How the stage shows its picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageView {
    /// `None` fits the picture to the stage; `Some(z)` is `z` screen pixels
    /// to a pixel of the render, 1.0 being 100%.
    zoom: Option<f32>,
    /// The point of the picture at the middle of the stage, as a fraction of
    /// its width and height.
    centre: Vec2,
}

impl Default for StageView {
    fn default() -> Self {
        StageView { zoom: None, centre: Vec2::splat(0.5) }
    }
}

/// Where the stage draws a picture, and how large.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// The whole picture, in points; often larger than the stage.
    pub rect: Rect,
    /// Screen pixels to a pixel of the render.
    pub scale: f32,
}

impl StageView {
    /// Whether it fits the picture to the stage.
    pub fn fitted(&self) -> bool {
        self.zoom.is_none()
    }

    /// Its zoom, screen pixels to a pixel of the render; `None` fitted.
    pub fn zoom(&self) -> Option<f32> {
        self.zoom
    }

    /// The point of the picture at the middle of the stage, as asked: the
    /// picture's edges hold it when it is drawn.
    pub fn centre(&self) -> Vec2 {
        self.centre
    }

    /// Zoom to `zoom` (`None` to fit) as a script asks, about the middle.
    pub fn set_zoom(&mut self, zoom: Option<f32>) {
        match zoom {
            Some(zoom) => self.zoom = Some(zoom.clamp(STEPS[0], MAX_ZOOM)),
            None => self.fit(),
        }
    }

    /// Look at the point `centre` of the picture, fractions of its width
    /// and height.
    pub fn look_at(&mut self, centre: Vec2) {
        self.centre = Vec2::new(centre.x.clamp(0.0, 1.0), centre.y.clamp(0.0, 1.0));
    }

    /// The scale at which `image` (in pixels) fits `stage` (in points).
    pub fn fit_scale(stage: Rect, image: [usize; 2], pixels_per_point: f32) -> f32 {
        let [width, height] = image.map(|side| side.max(1) as f32);
        (stage.width().max(1.0) * pixels_per_point / width).min(stage.height().max(1.0) * pixels_per_point / height)
    }

    /// The scale it shows `image` at.
    pub fn scale(&self, stage: Rect, image: [usize; 2], pixels_per_point: f32) -> f32 {
        self.zoom.unwrap_or_else(|| Self::fit_scale(stage, image, pixels_per_point))
    }

    /// The furthest out it goes: the fit, or 100% for a picture smaller than
    /// the stage, which Fit enlarges.
    fn min_zoom(stage: Rect, image: [usize; 2], pixels_per_point: f32) -> f32 {
        Self::fit_scale(stage, image, pixels_per_point).min(1.0)
    }

    /// Where `image` goes on `stage`.
    pub fn place(&self, stage: Rect, image: [usize; 2], pixels_per_point: f32) -> Placed {
        let scale = self.scale(stage, image, pixels_per_point);
        let size = Vec2::new(image[0].max(1) as f32, image[1].max(1) as f32) * scale / pixels_per_point;
        if self.fitted() {
            return Placed { rect: Rect::from_center_size(stage.center(), size), scale };
        }
        let centre = clamp_centre(self.centre, stage.size(), size);
        let min = stage.center() - centre * size;
        // On whole pixels of the screen, so that at 100% each pixel of the
        // render is one of the screen's rather than blended across two.
        let snap = |v: f32| (v * pixels_per_point).round() / pixels_per_point;
        Placed { rect: Rect::from_min_size(Pos2::new(snap(min.x), snap(min.y)), size), scale }
    }

    /// Zoom to `scale`, keeping the point of the picture under `about`
    /// where it is. Down to the fit (or 100%), up to [`MAX_ZOOM`]; at the
    /// fit, it fits.
    pub fn zoom_to(&mut self, stage: Rect, image: [usize; 2], pixels_per_point: f32, scale: f32, about: Pos2) {
        if !scale.is_finite() {
            return;
        }
        let fit = Self::fit_scale(stage, image, pixels_per_point);
        let scale = scale.clamp(Self::min_zoom(stage, image, pixels_per_point), MAX_ZOOM);
        if (scale - fit).abs() <= fit * 1e-3 {
            *self = StageView::default();
            return;
        }
        let before = self.place(stage, image, pixels_per_point);
        // The point under `about`, as a fraction of the picture.
        let under = (about - before.rect.min) / before.rect.size();
        let size = Vec2::new(image[0].max(1) as f32, image[1].max(1) as f32) * scale / pixels_per_point;
        let min = about - under * size;
        self.zoom = Some(scale);
        self.centre = clamp_centre((stage.center() - min) / size, stage.size(), size);
    }

    /// Zoom by `factor` about `about`: a wheel's turn, a pinch.
    pub fn zoom_by(&mut self, stage: Rect, image: [usize; 2], pixels_per_point: f32, factor: f32, about: Pos2) {
        let scale = self.scale(stage, image, pixels_per_point) * factor;
        self.zoom_to(stage, image, pixels_per_point, scale, about);
    }

    /// The next zoom in or out of [`STEPS`], about `about`. The fit is a
    /// step too: stepping across it lands on it.
    pub fn step(&mut self, stage: Rect, image: [usize; 2], pixels_per_point: f32, inwards: bool, about: Pos2) {
        let now = self.scale(stage, image, pixels_per_point);
        let fit = Self::fit_scale(stage, image, pixels_per_point);
        let mut next = if inwards {
            STEPS.iter().copied().find(|step| *step > now * 1.001).unwrap_or(MAX_ZOOM)
        } else {
            STEPS.iter().rev().copied().find(|step| *step < now / 1.001).unwrap_or(0.0)
        };
        if (now < fit * 0.999 && next > fit) || (now > fit * 1.001 && next < fit) {
            next = fit;
        }
        self.zoom_to(stage, image, pixels_per_point, next, about);
    }

    /// Fit the picture to the stage.
    pub fn fit(&mut self) {
        *self = StageView::default();
    }

    /// 100%, about `about`.
    pub fn actual_size(&mut self, stage: Rect, image: [usize; 2], pixels_per_point: f32, about: Pos2) {
        self.zoom_to(stage, image, pixels_per_point, 1.0, about);
        // Even where the fit is 100%: asked for, it stays 100% as the window
        // is resized.
        if self.zoom.is_none() {
            self.zoom = Some(1.0);
        }
    }

    /// Whether it is at 100%.
    pub fn at_actual_size(&self, stage: Rect, image: [usize; 2], pixels_per_point: f32) -> bool {
        (self.scale(stage, image, pixels_per_point) - 1.0).abs() < 1e-3
    }

    /// Move the picture by `delta` points, as far as its edges allow.
    pub fn pan(&mut self, stage: Rect, image: [usize; 2], pixels_per_point: f32, delta: Vec2) {
        if self.fitted() {
            return;
        }
        let placed = self.place(stage, image, pixels_per_point);
        self.centre = clamp_centre(self.centre - delta / placed.rect.size(), stage.size(), placed.rect.size());
    }

    /// Whether the picture is larger than the stage, and so can be dragged.
    pub fn pannable(&self, stage: Rect, image: [usize; 2], pixels_per_point: f32) -> bool {
        let placed = self.place(stage, image, pixels_per_point);
        !self.fitted() && (placed.rect.width() > stage.width() + 0.5 || placed.rect.height() > stage.height() + 0.5)
    }

    /// Whether its pixels are drawn as hard-edged squares: zoomed past 100%.
    pub fn crisp(&self) -> bool {
        self.zoom.is_some_and(|zoom| zoom > 1.0 + 1e-3)
    }
}

/// The zoom as the toolbar says it: "63%".
pub fn percent(scale: f32) -> String {
    let percent = scale * 100.0;
    if percent < 9.95 {
        format!("{percent:.1}%")
    } else {
        format!("{percent:.0}%")
    }
}

/// `centre` held so the picture's edges stay at the stage's: centred along
/// a side where it is smaller than the stage.
fn clamp_centre(centre: Vec2, stage: Vec2, size: Vec2) -> Vec2 {
    let axis = |centre: f32, stage: f32, size: f32| {
        if size <= stage || size <= 0.0 {
            0.5
        } else {
            let half = stage / size * 0.5;
            centre.clamp(half, 1.0 - half)
        }
    };
    Vec2::new(axis(centre.x, stage.x, size.x), axis(centre.y, stage.y, size.y))
}

/// What the toolbar, the keys or the mouse asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ask {
    ZoomIn,
    ZoomOut,
    Fit,
    ActualSize,
    /// Pause (`true`) or carry on.
    Pause(bool),
    Stop,
}

/// The keys of an image viewer, while nothing is being typed: `+` and `-`,
/// `0` or `F` to fit, `1` for 100%. The Render window has the keyboard to
/// itself when it is a window of its own; inside the main window, where its
/// keys and the main window's are one, only while the pointer is over the
/// picture (`over`).
pub fn keys(ui: &egui::Ui, over: bool) -> Option<Ask> {
    let own_window = ui.ctx().viewport_id() != egui::ViewportId::ROOT;
    if ui.ctx().wants_keyboard_input() || !(own_window || over) {
        return None;
    }
    use egui::{Key, Modifiers};
    ui.input_mut(|input| {
        let mut pressed = |key: Key| input.consume_key(Modifiers::NONE, key);
        if pressed(Key::Plus) || pressed(Key::Equals) {
            Some(Ask::ZoomIn)
        } else if pressed(Key::Minus) {
            Some(Ask::ZoomOut)
        } else if pressed(Key::Num0) || pressed(Key::F) {
            Some(Ask::Fit)
        } else if pressed(Key::Num1) {
            Some(Ask::ActualSize)
        } else {
            None
        }
    })
}

/// The wheel and the mouse over the stage: scroll or pinch to zoom about the
/// pointer, drag to look around a picture larger than the stage (the middle
/// button always; the left too, unless `left_is_taken`, as by the A/B
/// wipe's split), double-click for 100% there and back to Fit.
pub fn mouse(view: &mut StageView, ui: &egui::Ui, response: &egui::Response, stage: Rect, image: [usize; 2], left_is_taken: bool) -> bool {
    let ppp = ui.ctx().pixels_per_point();
    let before = *view;
    if response.hovered() {
        if let Some(pointer) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            if scroll != 0.0 {
                view.zoom_by(stage, image, ppp, WHEEL_STEP.powf(scroll / crate::orbit::POINTS_PER_NOTCH), pointer);
            }
            if pinch != 1.0 {
                view.zoom_by(stage, image, ppp, pinch, pointer);
            }
        }
    }
    if response.double_clicked_by(egui::PointerButton::Primary) && !left_is_taken {
        match response.interact_pointer_pos() {
            Some(pointer) if view.fitted() => view.actual_size(stage, image, ppp, pointer),
            _ => view.fit(),
        }
    }
    let pannable = view.pannable(stage, image, ppp);
    let dragging = response.dragged_by(egui::PointerButton::Middle) || (!left_is_taken && response.dragged_by(egui::PointerButton::Primary));
    if dragging && pannable {
        view.pan(stage, image, ppp, response.drag_delta());
    }
    if pannable && !left_is_taken {
        if dragging {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    *view != before
}

/// Carry out what was asked of the view; Pause and Stop are the caller's.
pub fn apply(view: &mut StageView, ask: Ask, stage: Rect, image: [usize; 2], pixels_per_point: f32) {
    match ask {
        Ask::ZoomIn => view.step(stage, image, pixels_per_point, true, stage.center()),
        Ask::ZoomOut => view.step(stage, image, pixels_per_point, false, stage.center()),
        Ask::Fit => view.fit(),
        Ask::ActualSize => view.actual_size(stage, image, pixels_per_point, stage.center()),
        Ask::Pause(_) | Ask::Stop => {}
    }
}

/// A render in progress, as the toolbar's Pause and Stop see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transport {
    pub paused: bool,
    /// Why it cannot be paused, when it cannot.
    pub unpausable: Option<&'static str>,
}

/// What the toolbar shows of the zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    pub scale: f32,
    pub fitted: bool,
    pub actual: bool,
}

/// A tool button over the picture, and its island's thickness, as the
/// viewport's.
const TOOL: f32 = 28.0;
const ISLAND: f32 = TOOL + 8.0;
const PAD: f32 = 4.0;
const GAP: f32 = 9.0;
const LABEL: f32 = 48.0;

/// The toolbar, floating at the foot of `stage`: zoom out, the zoom, zoom
/// in, Fit and 100% when there is a picture to zoom (`zoom`), and Pause and
/// Stop while a render runs (`transport`).
pub fn toolbar(ui: &mut egui::Ui, stage: Rect, zoom: Option<Zoom>, transport: Option<Transport>) -> Option<Ask> {
    if zoom.is_none() && transport.is_none() {
        return None;
    }
    let mut width = PAD * 2.0;
    if zoom.is_some() {
        width += TOOL * 4.0 + LABEL + GAP;
    }
    if transport.is_some() {
        width += TOOL * 2.0 + if zoom.is_some() { GAP } else { 0.0 };
    }
    let island = Rect::from_min_size(
        Pos2::new(stage.center().x - width * 0.5, stage.bottom() - 12.0 - ISLAND),
        egui::vec2(width, ISLAND),
    );
    let painter = ui.painter().clone();
    icons::island(&painter, island);
    // Sensing drags as well, so a drag begun on the island does not pan.
    let sense = egui::Sense::click_and_drag();
    let _ = ui.interact(island, ui.id().with("render-toolbar"), sense);
    let y = island.top() + PAD;
    let mut x = island.left() + PAD;
    let mut ask = None;
    let button = |ui: &mut egui::Ui, x: &mut f32, name: &str, icon: Icon, on: bool, enabled: bool| {
        let at = Rect::from_min_size(Pos2::new(*x, y), Vec2::splat(TOOL));
        *x += TOOL;
        let response = ui.interact(at, ui.id().with(("render-tool", name)), sense);
        icons::paint_button(ui, at, &response, icon, if on { Tone::On } else { Tone::Quiet }, enabled);
        response
    };
    let divider = |x: &mut f32| {
        icons::divider(&painter, Pos2::new(*x + GAP * 0.5, island.top() + 9.0), Pos2::new(*x + GAP * 0.5, island.bottom() - 9.0));
        *x += GAP;
    };
    if let Some(zoom) = zoom {
        let out = button(ui, &mut x, "zoom-out", Icon::ZoomOut, false, true);
        if icons::tip(out, "Zoom out", "Further from the picture. The wheel zooms about the pointer.", "-").clicked() {
            ask = Some(Ask::ZoomOut);
        }
        let label = Rect::from_min_size(Pos2::new(x, y), egui::vec2(LABEL, TOOL));
        x += LABEL;
        let response = ui.interact(label, ui.id().with(("render-tool", "zoom")), egui::Sense::click());
        painter.text(label.center(), egui::Align2::CENTER_CENTER, percent(zoom.scale), egui::FontId::proportional(12.5), theme::p().text);
        let response = icons::tip(
            response,
            "Zoom",
            "At 100%, one pixel of the render is one pixel of the screen. Drag to look around, double-click for 100% and back.",
            "",
        );
        if response.double_clicked() {
            ask = Some(if zoom.fitted { Ask::ActualSize } else { Ask::Fit });
        }
        let closer = button(ui, &mut x, "zoom-in", Icon::ZoomIn, false, zoom.scale < MAX_ZOOM * 0.999);
        if icons::tip(closer, "Zoom in", "Closer, to judge the render's pixels", "+").clicked() && zoom.scale < MAX_ZOOM * 0.999 {
            ask = Some(Ask::ZoomIn);
        }
        divider(&mut x);
        let fit = button(ui, &mut x, "fit", Icon::FrameAll, zoom.fitted, true);
        if icons::tip(fit, "Fit", "The whole picture in the window", "0 / F").clicked() {
            ask = Some(Ask::Fit);
        }
        // Written out: no icon says 100% as plainly.
        let at = Rect::from_min_size(Pos2::new(x, y), Vec2::splat(TOOL));
        x += TOOL;
        let response = ui.interact(at, ui.id().with(("render-tool", "actual")), sense);
        text_button(ui, at, &response, "1:1", zoom.actual);
        if icons::tip(response, "100%", "One pixel of the render to one pixel of the screen, whatever the display's scaling", "1").clicked() {
            ask = Some(Ask::ActualSize);
        }
    }
    if let Some(transport) = transport {
        if zoom.is_some() {
            divider(&mut x);
        }
        let pausable = transport.unpausable.is_none() || transport.paused;
        let (icon, title, detail) = match transport.paused {
            true => (Icon::Play, "Resume", "Carry on rendering from the samples it has"),
            false => (Icon::Pause, "Pause", "Hold the render after the pass it is on, and the queue behind it. Nothing is lost: Resume carries on from the same samples."),
        };
        let response = button(ui, &mut x, "pause", icon, transport.paused, pausable);
        let detail = transport.unpausable.filter(|_| !transport.paused).unwrap_or(detail);
        if icons::tip(response, title, detail, "").clicked() && pausable {
            ask = Some(Ask::Pause(!transport.paused));
        }
        let stop = button(ui, &mut x, "stop", Icon::Stop, false, true);
        if icons::tip(stop, "Stop", "End the render and save the picture as it is now. Renders waiting behind it start.", "").clicked() {
            ask = Some(Ask::Stop);
        }
    }
    ask
}

/// A button showing a few characters where an icon would be, as the
/// viewport's tools are drawn.
fn text_button(ui: &egui::Ui, rect: Rect, response: &egui::Response, text: &str, on: bool) {
    let p = theme::p();
    let hovered = response.hovered();
    let ease = ui.ctx().animate_bool_with_time(response.id.with("ease"), hovered, 0.1);
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, 7.0, p.accent_fill);
    } else if response.is_pointer_button_down_on() {
        painter.rect_filled(rect, 7.0, p.pressed.gamma_multiply(0.85));
    } else if ease > 0.0 {
        painter.rect_filled(rect, 7.0, p.hover.gamma_multiply(0.7 * ease));
    }
    let colour = if on { p.accent_text } else { theme::mix(p.icon, p.icon_hover, ease) };
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, theme::strong(12.0), colour);
}

/// "Paused", at the top of the stage, while a render is held, or about to
/// be: plain enough to see from across the room.
pub fn paused_badge(ui: &egui::Ui, stage: Rect, held: bool) {
    let p = theme::p();
    let text = if held { "Paused" } else { "Pausing after this pass…" };
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(text.to_string(), theme::strong(13.0), p.busy);
    let size = egui::vec2(16.0 + 12.0 + galley.size().x + 14.0, 28.0);
    let pill = Rect::from_min_size(Pos2::new(stage.center().x - size.x * 0.5, stage.top() + 12.0), size);
    icons::island(painter, pill);
    painter.rect_filled(pill, 11.0, p.busy.gamma_multiply(0.14));
    icons::paint(painter, Rect::from_center_size(Pos2::new(pill.left() + 18.0, pill.center().y), Vec2::splat(13.0)), Icon::Pause, p.busy);
    painter.galley(Pos2::new(pill.left() + 30.0, pill.center().y - galley.size().y * 0.5), galley, egui::Color32::PLACEHOLDER);
}

// ---- pictures ---------------------------------------------------------------

/// A picture as the stage draws it, its pixels kept so it can be drawn
/// smooth or crisp, in tiles the graphics card can hold: a 10000-pixel
/// poster is more than one texture can be.
pub struct Picture {
    size: [usize; 2],
    tiles: Vec<Tile>,
    crisp: bool,
}

struct Tile {
    /// Its corner in the picture, in pixels.
    at: [usize; 2],
    image: Arc<egui::ColorImage>,
    texture: egui::TextureHandle,
}

fn options(crisp: bool) -> egui::TextureOptions {
    if crisp {
        egui::TextureOptions { magnification: egui::TextureFilter::Nearest, ..egui::TextureOptions::LINEAR }
    } else {
        egui::TextureOptions::LINEAR
    }
}

impl Picture {
    pub fn new(ctx: &egui::Context, name: &str, image: egui::ColorImage, crisp: bool) -> Picture {
        let size = image.size;
        let side = ctx.input(|i| i.max_texture_side).clamp(256, 8192);
        let tiles = if size[0] <= side && size[1] <= side {
            let image = Arc::new(image);
            let texture = ctx.load_texture(name, egui::ImageData::Color(image.clone()), options(crisp));
            vec![Tile { at: [0, 0], image, texture }]
        } else {
            let mut tiles = Vec::new();
            for y0 in (0..size[1]).step_by(side) {
                for x0 in (0..size[0]).step_by(side) {
                    let (w, h) = (side.min(size[0] - x0), side.min(size[1] - y0));
                    let mut pixels = Vec::with_capacity(w * h);
                    for y in y0..y0 + h {
                        pixels.extend_from_slice(&image.pixels[y * size[0] + x0..y * size[0] + x0 + w]);
                    }
                    let tile = Arc::new(egui::ColorImage { size: [w, h], pixels });
                    let texture = ctx.load_texture(format!("{name}-{x0}-{y0}"), egui::ImageData::Color(tile.clone()), options(crisp));
                    tiles.push(Tile { at: [x0, y0], image: tile, texture });
                }
            }
            tiles
        };
        Picture { size, tiles, crisp }
    }

    /// Its size in pixels.
    pub fn size(&self) -> [usize; 2] {
        self.size
    }

    /// Drawn with hard-edged pixels, or smoothed.
    pub fn set_crisp(&mut self, crisp: bool) {
        if crisp == self.crisp {
            return;
        }
        self.crisp = crisp;
        for tile in &mut self.tiles {
            tile.texture.set(egui::ImageData::Color(tile.image.clone()), options(crisp));
        }
    }

    /// Draw the part of it `part` (fractions of its width and height) where
    /// that part falls when the whole is `whole`.
    pub fn paint(&self, painter: &egui::Painter, whole: Rect, part: Rect) {
        let [width, height] = self.size.map(|side| side.max(1) as f32);
        for tile in &self.tiles {
            let [tw, th] = tile.image.size;
            let span = Rect::from_min_size(
                Pos2::new(tile.at[0] as f32 / width, tile.at[1] as f32 / height),
                egui::vec2(tw as f32 / width, th as f32 / height),
            );
            let shown = span.intersect(part);
            if !shown.is_positive() {
                continue;
            }
            let on_screen = Rect::from_min_max(whole.lerp_inside(shown.min.to_vec2()), whole.lerp_inside(shown.max.to_vec2()));
            if !painter.clip_rect().intersects(on_screen) {
                continue;
            }
            let uv = Rect::from_min_max(
                ((shown.min - span.min) / span.size()).to_pos2(),
                ((shown.max - span.min) / span.size()).to_pos2(),
            );
            painter.image(tile.texture.id(), on_screen, uv, egui::Color32::WHITE);
        }
    }
}

/// The part of a picture that is all of it.
pub fn whole() -> Rect {
    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage() -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 50.0), egui::vec2(800.0, 450.0))
    }

    const IMAGE: [usize; 2] = [3840, 2160];

    /// Fit shows the whole picture, as large as the stage allows, centred;
    /// its scale is screen pixels, so twice as large on a display at 200%.
    #[test]
    fn fit_fills_the_stage_at_the_pictures_shape() {
        let view = StageView::default();
        let placed = view.place(stage(), IMAGE, 1.0);
        assert!((placed.rect.width() - 800.0).abs() < 1e-3 && (placed.rect.height() - 450.0).abs() < 1e-3, "{placed:?}");
        assert_eq!(placed.rect.center(), stage().center());
        assert!((placed.scale - 800.0 / 3840.0).abs() < 1e-6);
        assert_eq!(percent(placed.scale), "21%");
        let retina = view.place(stage(), IMAGE, 2.0);
        assert_eq!(retina.rect, placed.rect, "the same points");
        assert!((retina.scale - 2.0 * placed.scale).abs() < 1e-6, "twice the pixels");
        // A tall picture fills the height.
        let tall = view.place(stage(), [1080, 1920], 1.0);
        assert!((tall.rect.height() - 450.0).abs() < 1e-3 && tall.rect.width() < 800.0);
    }

    /// 100% is one pixel of the render to one pixel of the screen: a
    /// 1920-pixel render is 1280 points across on a display at 150%, and
    /// sits on whole pixels.
    #[test]
    fn actual_size_is_one_pixel_to_a_screen_pixel() {
        for ppp in [1.0, 1.25, 1.5, 2.0] {
            let mut view = StageView::default();
            view.actual_size(stage(), [1920, 1080], ppp, stage().center());
            let placed = view.place(stage(), [1920, 1080], ppp);
            assert!((placed.scale - 1.0).abs() < 1e-6);
            assert!((placed.rect.width() * ppp - 1920.0).abs() < 1e-2, "{ppp}: {placed:?}");
            assert!((placed.rect.height() * ppp - 1080.0).abs() < 1e-2);
            for v in [placed.rect.min.x, placed.rect.min.y] {
                assert!(((v * ppp) - (v * ppp).round()).abs() < 1e-3, "{ppp}: on a whole pixel, {v}");
            }
            assert!(view.at_actual_size(stage(), [1920, 1080], ppp));
            assert!(!view.crisp(), "drawn on whole pixels, not as squares");
            assert_eq!(percent(placed.scale), "100%");
        }
    }

    /// Zooming about a point keeps the point of the picture under it where
    /// it was, the wheel's way.
    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let ppp = 1.25;
        let mut view = StageView::default();
        view.actual_size(stage(), IMAGE, ppp, stage().center());
        let pointer = Pos2::new(300.0, 200.0);
        let under = |view: &StageView| {
            let placed = view.place(stage(), IMAGE, ppp);
            (pointer - placed.rect.min) / placed.rect.size()
        };
        let before = under(&view);
        for factor in [1.7, 3.0, 0.6] {
            view.zoom_by(stage(), IMAGE, ppp, factor, pointer);
            let after = under(&view);
            // Within a pixel of the screen, which the placement snaps to.
            let size = view.place(stage(), IMAGE, ppp).rect.size();
            assert!(((after - before) * size).length() <= 1.0 / ppp + 1e-3, "{factor}: {before:?} became {after:?}");
        }
    }

    /// Zoomed in, the picture's edges stop at the stage's: it cannot be
    /// dragged off. Smaller than the stage along a side, it is centred.
    #[test]
    fn panning_stops_at_the_pictures_edges() {
        let mut view = StageView::default();
        view.zoom_to(stage(), IMAGE, 1.0, 1.0, stage().center());
        view.pan(stage(), IMAGE, 1.0, egui::vec2(1e6, 1e6));
        let placed = view.place(stage(), IMAGE, 1.0);
        assert_eq!(placed.rect.min, stage().min, "dragged right and down as far as it goes: its top left corner shows");
        view.pan(stage(), IMAGE, 1.0, egui::vec2(-1e6, -1e6));
        let placed = view.place(stage(), IMAGE, 1.0);
        assert!((placed.rect.max - stage().max).length() < 1.0, "and the other way, its bottom right: {placed:?}");
        // A wide strip at 100% is taller than nothing: centred up and down.
        let strip = [4000, 200];
        view.actual_size(stage(), strip, 1.0, stage().center());
        view.pan(stage(), strip, 1.0, egui::vec2(0.0, 500.0));
        let placed = view.place(stage(), strip, 1.0);
        assert!((placed.rect.center().y - stage().center().y).abs() < 1.0, "{placed:?}");
        assert!(view.pannable(stage(), strip, 1.0));
        // At Fit there is nothing to drag.
        view.fit();
        assert!(!view.pannable(stage(), strip, 1.0));
    }

    /// Out as far as the fit, in as far as [`MAX_ZOOM`]; at the fit, it
    /// fits, and follows the stage. A picture smaller than the stage goes
    /// down to 100%, below its fit.
    #[test]
    fn the_zoom_is_held_between_the_fit_and_its_limit() {
        let mut view = StageView::default();
        view.zoom_to(stage(), IMAGE, 1.0, 0.01, stage().center());
        assert!(view.fitted(), "zoomed out past the fit, it fits");
        view.zoom_to(stage(), IMAGE, 1.0, 1000.0, stage().center());
        assert_eq!(view.scale(stage(), IMAGE, 1.0), MAX_ZOOM);
        assert!(view.crisp());
        let small = [400, 300];
        let fit = StageView::fit_scale(stage(), small, 1.0);
        assert!(fit > 1.0);
        view.zoom_to(stage(), small, 1.0, 0.1, stage().center());
        assert_eq!(view.scale(stage(), small, 1.0), 1.0, "a small picture goes down to 100%");
        assert!(!view.fitted());
        view.zoom_to(stage(), small, 1.0, fit, stage().center());
        assert!(view.fitted());
    }

    /// The buttons and keys step through an image viewer's zooms, landing
    /// on the fit when they cross it.
    #[test]
    fn steps_land_on_the_fit_and_round_numbers() {
        let mut view = StageView::default();
        let at = |view: &StageView| percent(view.scale(stage(), IMAGE, 1.0));
        assert_eq!(at(&view), "21%");
        let mut seen = Vec::new();
        for _ in 0..5 {
            view.step(stage(), IMAGE, 1.0, true, stage().center());
            seen.push(at(&view));
        }
        assert_eq!(seen, ["25%", "33%", "50%", "67%", "100%"]);
        view.step(stage(), IMAGE, 1.0, true, stage().center());
        assert_eq!(at(&view), "150%");
        assert!(view.crisp(), "past 100%, pixels are squares");
        for _ in 0..6 {
            view.step(stage(), IMAGE, 1.0, false, stage().center());
        }
        assert!(view.fitted(), "and back out to the fit: {}", at(&view));
        view.step(stage(), IMAGE, 1.0, false, stage().center());
        assert!(view.fitted(), "and no further");
    }

    /// The fit follows the stage as the window is resized; a zoom keeps
    /// its scale.
    #[test]
    fn fit_follows_the_window_and_a_zoom_keeps_its_scale() {
        let wide = Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0));
        let fitted = StageView::default();
        assert!(fitted.scale(wide, IMAGE, 1.0) > fitted.scale(stage(), IMAGE, 1.0));
        let mut zoomed = StageView::default();
        zoomed.zoom_to(stage(), IMAGE, 1.0, 0.5, stage().center());
        assert_eq!(zoomed.scale(wide, IMAGE, 1.0), 0.5);
    }

    #[test]
    fn small_zooms_keep_a_decimal() {
        assert_eq!(percent(0.05), "5.0%");
        assert_eq!(percent(1.0 / 3.0), "33%");
        assert_eq!(percent(32.0), "3200%");
    }

    /// A picture larger than a texture can be is drawn in tiles, every
    /// pixel kept, and each tile is drawn where its part of the picture is.
    #[test]
    fn a_large_picture_is_drawn_in_tiles() {
        let ctx = egui::Context::default();
        // The smallest side egui's own font atlas allows.
        let _ = ctx.run(egui::RawInput { max_texture_side: Some(1024), ..Default::default() }, |_| {});
        let (w, h) = (2500, 1200);
        let image = egui::ColorImage { size: [w, h], pixels: (0..w * h).map(|i| egui::Color32::from_gray((i % w * 255 / w) as u8)).collect() };
        let picture = Picture::new(&ctx, "probe", image, false);
        assert_eq!(picture.size(), [2500, 1200]);
        assert_eq!(picture.tiles.len(), 3 * 2, "three across, two down");
        let pixels: usize = picture.tiles.iter().map(|t| t.image.size[0] * t.image.size[1]).sum();
        assert_eq!(pixels, w * h, "every pixel, once");
        let corner = picture.tiles.iter().find(|t| t.at == [2048, 1024]).expect("the last tile");
        assert_eq!(corner.image.size, [452, 176]);
        assert_eq!(corner.image.pixels[0], egui::Color32::from_gray((2048 * 255 / w) as u8), "its own pixels");
        // Drawn over the right half only, the left tiles are left out and
        // the middle one is cut.
        let output = ctx.run(Default::default(), |ctx| {
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("probe"));
            let painter = egui::Painter::new(ctx.clone(), layer, Rect::EVERYTHING);
            let half = Rect::from_min_max(Pos2::new(0.5, 0.0), Pos2::new(1.0, 1.0));
            picture.paint(&painter, Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 300.0)), half);
        });
        let drawn: Vec<Rect> = output.shapes.iter().map(|c| c.shape.visual_bounding_rect()).collect();
        assert_eq!(drawn.len(), 4, "two tiles across in each row: {drawn:?}");
        assert!(drawn.iter().all(|r| r.left() >= 300.0 - 1e-3), "{drawn:?}");
    }
}
