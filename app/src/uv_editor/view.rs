//! The canvas's view of the layout: how far in it is, and where.
//!
//! The layout is drawn as the texture sees it: `v` down, as an image's rows
//! are, so artwork placed square looks the way it does in a picture. At a
//! zoom of 1 (100%) the unit square fills nine tenths of the canvas's
//! shorter side.

use tequila_core::math::{vec2, Vec2};

pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 400.0;

/// How much of the canvas the unit square fills at 100%, and a framed
/// selection fills.
const MARGIN: f32 = 0.9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f32,
    /// How far the square's middle is from the canvas's, in points.
    pub pan: egui::Vec2,
}

impl Default for View {
    fn default() -> Self {
        View { zoom: 1.0, pan: egui::Vec2::ZERO }
    }
}

/// The view on one canvas, this frame: layout to screen and back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    /// Screen points per layout unit.
    pub scale: f32,
    /// Where the layout's origin is on screen.
    pub origin: egui::Pos2,
}

impl Xform {
    pub fn to_screen(&self, p: Vec2) -> egui::Pos2 {
        egui::pos2(self.origin.x + p.x * self.scale, self.origin.y + p.y * self.scale)
    }

    pub fn to_layout(&self, p: egui::Pos2) -> Vec2 {
        vec2((p.x - self.origin.x) / self.scale, (p.y - self.origin.y) / self.scale)
    }

    /// A length on screen, in layout units.
    pub fn layout_length(&self, points: f32) -> f32 {
        points / self.scale
    }

    pub fn rect(&self, low: Vec2, high: Vec2) -> egui::Rect {
        egui::Rect::from_two_pos(self.to_screen(low), self.to_screen(high))
    }

    /// The part of the layout the canvas shows.
    pub fn visible(&self, canvas: egui::Rect) -> (Vec2, Vec2) {
        (self.to_layout(canvas.min), self.to_layout(canvas.max))
    }
}

impl View {
    /// Points per layout unit at zoom 1, on a canvas.
    fn base(canvas: egui::Rect) -> f32 {
        (canvas.width().min(canvas.height()).max(120.0)) * MARGIN
    }

    pub fn xform(&self, canvas: egui::Rect) -> Xform {
        let scale = View::base(canvas) * self.zoom;
        let origin = canvas.center() + self.pan - egui::vec2(scale * 0.5, scale * 0.5);
        Xform { scale, origin }
    }

    /// Zoomed by `factor` about a point of the screen, so what is under it
    /// stays there.
    pub fn zoom_about(&mut self, canvas: egui::Rect, factor: f32, about: egui::Pos2) {
        let before = self.xform(canvas);
        let fixed = before.to_layout(about);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.xform(canvas);
        self.pan += about - after.to_screen(fixed);
    }

    /// Set to `zoom`, keeping the middle of the canvas where it is.
    pub fn zoom_to(&mut self, canvas: egui::Rect, zoom: f32) {
        let factor = zoom.clamp(MIN_ZOOM, MAX_ZOOM) / self.zoom;
        self.zoom_about(canvas, factor, canvas.center());
    }

    /// Framed on a box of the layout, which fills nine tenths of the canvas
    /// across or down, whichever it reaches first.
    pub fn frame(&mut self, canvas: egui::Rect, low: Vec2, high: Vec2) {
        let size = (high - low).max(Vec2::splat(1e-4));
        let fit = (canvas.width() / size.x).min(canvas.height() / size.y) * MARGIN;
        self.zoom = (fit / View::base(canvas)).clamp(MIN_ZOOM, MAX_ZOOM);
        let xform = self.xform(canvas);
        let middle = (low + high) * 0.5;
        self.pan += canvas.center() - xform.to_screen(middle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(800.0, 600.0))
    }

    #[test]
    fn the_view_maps_the_layout_there_and_back() {
        let view = View::default();
        let x = view.xform(canvas());
        assert!((x.scale - 540.0).abs() < 1e-3, "nine tenths of the shorter side");
        assert!((x.to_screen(vec2(0.5, 0.5)) - canvas().center()).length() < 1e-3);
        let p = vec2(0.2, -0.7);
        assert!((x.to_layout(x.to_screen(p)) - p).length() < 1e-5);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let mut view = View::default();
        let at = egui::pos2(300.0, 200.0);
        let before = view.xform(canvas()).to_layout(at);
        view.zoom_about(canvas(), 3.0, at);
        let after = view.xform(canvas()).to_layout(at);
        assert!((before - after).length() < 1e-5);
        assert!((view.zoom - 3.0).abs() < 1e-6);
    }

    #[test]
    fn framing_fills_the_canvas_with_the_box() {
        let mut view = View::default();
        view.frame(canvas(), vec2(0.2, 0.3), vec2(0.4, 0.4));
        let x = view.xform(canvas());
        let shown = x.rect(vec2(0.2, 0.3), vec2(0.4, 0.4));
        assert!((shown.center() - canvas().center()).length() < 1e-3);
        assert!((shown.width() - 720.0).abs() < 1e-2, "{}", shown.width());
    }
}
