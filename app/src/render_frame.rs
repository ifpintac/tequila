//! The render frame: the viewport shown at the render's own shape.
//!
//! The camera keeps its horizontal field of view whatever the image's shape
//! (`Camera::sensor_height_mm`), so a viewport of one shape and a render of
//! another frame the product differently: a 16:9 render from a 4:3 view cuts
//! off the top and bottom of what was on screen, and a tall render shows
//! more than the view did. With the frame on, the picture is drawn at the
//! render's shape, centred, and the rest of the viewport is left empty, so
//! the view holds exactly what the render will.

use crate::theme;
use eframe::egui;
use tequila_scene::render::RenderSettings;

/// The largest rectangle of a `width` by `height` image that fits in `area`,
/// centred, in whole points so the picture is never stretched by a fraction
/// of one.
pub fn fit(area: egui::Rect, width: usize, height: usize) -> egui::Rect {
    if width == 0 || height == 0 || area.width() < 1.0 || area.height() < 1.0 {
        return area;
    }
    let aspect = width as f32 / height as f32;
    let (w, h) = if area.width() / area.height() > aspect {
        ((area.height() * aspect).floor(), area.height().floor())
    } else {
        (area.width().floor(), (area.width() / aspect).floor())
    };
    let size = egui::vec2(w.max(16.0), h.max(16.0));
    egui::Rect::from_min_size((area.center() - size * 0.5).round(), size)
}

/// The empty bands either side of the picture, a hairline round it, and the
/// render's size beside it where there is room.
pub fn draw(painter: &egui::Painter, area: egui::Rect, frame: egui::Rect, settings: &RenderSettings) {
    let palette = theme::p();
    let bands = [
        egui::Rect::from_min_max(area.min, egui::pos2(area.max.x, frame.min.y)),
        egui::Rect::from_min_max(egui::pos2(area.min.x, frame.max.y), area.max),
        egui::Rect::from_min_max(egui::pos2(area.min.x, frame.min.y), egui::pos2(frame.min.x, frame.max.y)),
        egui::Rect::from_min_max(egui::pos2(frame.max.x, frame.min.y), egui::pos2(area.max.x, frame.max.y)),
    ];
    for band in bands.into_iter().filter(|b| b.width() > 0.0 && b.height() > 0.0) {
        painter.rect_filled(band, 0.0, palette.chrome);
    }
    painter.rect_stroke(frame.expand(0.5), 0.0, egui::Stroke::new(1.0_f32, palette.line_strong));

    // Under the picture's bottom-right corner when the bands are above and
    // below, beside it when they are left and right.
    let text = format!("{} × {}", settings.width, settings.height);
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.0), palette.text_dim);
    let size = galley.size();
    let below = area.max.y - frame.max.y;
    let beside = area.max.x - frame.max.x;
    let at = if below >= size.y + 8.0 {
        Some(egui::pos2(frame.max.x - size.x - 8.0, frame.max.y + (below.min(28.0) - size.y) * 0.5))
    } else if beside >= size.x + 16.0 {
        Some(egui::pos2(frame.max.x + 8.0, frame.max.y - size.y))
    } else {
        None
    };
    if let Some(at) = at {
        painter.galley(at, galley, palette.text_dim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(300.0, 40.0), egui::vec2(w, h))
    }

    fn assert_shape(frame: egui::Rect, width: usize, height: usize) {
        let want = width as f32 / height as f32;
        let got = frame.width() / frame.height();
        assert!(
            (got - want).abs() < 2.0 / frame.height().min(frame.width()),
            "the frame is {} × {} ({got:.4}), not the render's {width} × {height} ({want:.4})",
            frame.width(),
            frame.height()
        );
    }

    /// The case people met: a 16:9 render from a squarer viewport. The frame
    /// spans the width, is the render's shape, and sits in the middle.
    #[test]
    fn a_wide_render_in_a_squarer_view_spans_its_width() {
        let view = area(1200.0, 900.0);
        let frame = fit(view, 1920, 1080);
        assert_shape(frame, 1920, 1080);
        assert_eq!(frame.width(), 1200.0);
        assert!(view.contains_rect(frame));
        assert!((frame.center().y - view.center().y).abs() <= 0.5);
    }

    /// A tall render in a wide viewport spans its height instead, and never
    /// reaches outside the viewport - the case where a render used to show
    /// more than the view did.
    #[test]
    fn a_tall_render_in_a_wide_view_spans_its_height() {
        let view = area(1500.0, 800.0);
        let frame = fit(view, 1080, 1350);
        assert_shape(frame, 1080, 1350);
        assert_eq!(frame.height(), 800.0);
        assert!(view.contains_rect(frame));
        assert!((frame.center().x - view.center().x).abs() <= 0.5);
    }

    /// Whole points: a picture stretched by a fraction of a point blurs.
    #[test]
    fn the_frame_lands_on_whole_points() {
        for (w, h, rw, rh) in [(1013.0, 717.5, 1920, 1080), (640.3, 999.9, 1000, 1000), (800.0, 600.0, 2480, 3508)] {
            let frame = fit(area(w, h), rw, rh);
            for v in [frame.min.x, frame.min.y, frame.width(), frame.height()] {
                assert_eq!(v, v.round(), "{v} is not a whole point in {frame:?}");
            }
            assert_shape(frame, rw, rh);
        }
    }

    /// A viewport already the render's shape is used whole.
    #[test]
    fn a_view_of_the_same_shape_is_used_whole() {
        let view = area(1600.0, 900.0);
        assert_eq!(fit(view, 1920, 1080), view);
    }
}
