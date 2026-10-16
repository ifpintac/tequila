//! The ground grid (`docs/04-UX-SPEC.md` §2: "**Ground grid** with real-world
//! spacing labels when enabled in the Camera tab").
//!
//! An overlay rather than geometry. A grid drawn into the scene would be traced,
//! shaded, denoised and accumulated — it would cast shadows, appear in
//! reflections, and restart the render every time it was toggled. What a grid is
//! for is telling a person how big something is, and that belongs on top of the
//! image, not in it.
//!
//! The spacing is the part worth getting right. A fixed spacing is useless twice
//! over: at 140 mm it is a wall of lines, at 40 m it is one line. So the spacing
//! is chosen from the camera's distance in 1-2-5 steps — the same progression a
//! ruler uses — and relabelled as the camera moves, which is what makes the grid
//! an answer to "how big is this" rather than decoration.

use eframe::egui;
use tequila_core::math::{vec3, Vec3};
use tequila_core::units::LengthUnit;
use tequila_render_core::camera::Camera;

/// How many lines to draw either side of the origin. Beyond this the lines are
/// closer together on screen than they are useful, and the count is what bounds
/// the work rather than the extent.
const HALF_COUNT: i32 = 12;

/// Draw the grid into `rect`, its spacing a round number in `unit`, the unit
/// lengths are shown in.
pub fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    camera: &Camera,
    target: Vec3,
    distance: f32,
    unit: LengthUnit,
) {
    // Chosen in the unit it is read in and drawn in the metres the scene is
    // in: 10 mm squares, rather than 0.01 m ones labelled in millimetres.
    let per_unit = unit.to_metres() as f32;
    let shown = nice_spacing(distance / per_unit);
    let spacing = shown * per_unit;
    if !(spacing > 0.0) {
        return;
    }
    let aspect = rect.width().max(1.0) / rect.height().max(1.0);

    // Centred on the camera's target rather than on the world origin, so the
    // grid is under whatever is being looked at rather than wherever the model
    // happens to have been authored.
    let centre = vec3(
        (target.x / spacing).round() * spacing,
        0.0,
        (target.z / spacing).round() * spacing,
    );
    let extent = spacing * HALF_COUNT as f32;

    let to_screen = |world: Vec3| -> Option<egui::Pos2> {
        let film = camera.project(world, aspect)?;
        Some(egui::pos2(
            rect.left() + film.x * rect.width(),
            rect.top() + film.y * rect.height(),
        ))
    };

    let faint = egui::Color32::from_rgba_unmultiplied(85, 95, 110, 65);
    let strong = egui::Color32::from_rgba_unmultiplied(100, 110, 125, 100);

    for i in -HALF_COUNT..=HALF_COUNT {
        let offset = i as f32 * spacing;
        // Every tenth line is drawn stronger, which is what lets someone count
        // decades across the frame without counting every line.
        let colour = if i % 10 == 0 { strong } else { faint };
        let width: f32 = if i % 10 == 0 { 1.2 } else { 0.7 };

        for axis in 0..2 {
            let (mut a, mut b) = if axis == 0 {
                (
                    centre + vec3(offset, 0.0, -extent),
                    centre + vec3(offset, 0.0, extent),
                )
            } else {
                (
                    centre + vec3(-extent, 0.0, offset),
                    centre + vec3(extent, 0.0, offset),
                )
            };
            // A segment with an endpoint behind the camera cannot be drawn from
            // its projected endpoints — the projection of a point behind the eye
            // lands on the wrong side of the frame and the line flies across the
            // screen. Dropping the segment is honest; clipping it properly is
            // worth doing when the grid gains a horizon fade.
            let forward = camera.forward();
            let near = (distance * 0.001).max(1e-5);
            let da = (a - camera.position()).dot(forward);
            let db = (b - camera.position()).dot(forward);
            if da < near && db < near { continue; }
            if da < near { a = a + (b - a) * ((near - da) / (db - da)); }
            if db < near { b = b + (a - b) * ((near - db) / ((a - camera.position()).dot(forward) - db)); }
            let (Some(a), Some(b)) = (to_screen(a), to_screen(b)) else {
                continue;
            };
            painter.line_segment([a, b], egui::Stroke::new(width, colour));
        }
    }

    // One label, on the spacing itself. A number per line would be unreadable
    // at any zoom that shows more than a handful of them.
    {
        painter.text(
            rect.right_top() + egui::vec2(-12.0, 12.0),
            egui::Align2::RIGHT_TOP,
            format!("Ground XZ · {} / square · Y = 0", format_spacing(shown, unit)),
            egui::FontId::proportional(11.0),
            egui::Color32::from_white_alpha(150),
        );
    }
}

/// A round spacing for the camera's distance, in 1-2-5 steps.
///
/// Aimed at roughly ten divisions across the view, which is the density at
/// which a grid reads as a measure rather than as a texture.
pub fn nice_spacing(distance: f32) -> f32 {
    let target = (distance.max(1e-6) / 10.0).max(1e-9);
    let decade = 10f32.powf(target.log10().floor());
    let steps = [1.0, 2.0, 5.0, 10.0];
    let mut best = decade;
    for step in steps {
        let candidate = decade * step;
        if candidate <= target * 2.0 {
            best = candidate;
        }
    }
    best
}

/// A spacing in `unit`, written the way a person reads it.
pub fn format_spacing(spacing: f32, unit: LengthUnit) -> String {
    let symbol = unit.symbol();
    let number = if spacing >= 100.0 {
        format!("{spacing:.0}")
    } else if spacing >= 1.0 {
        format!("{spacing:.3}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    } else {
        format!("{spacing:.4}")
    };
    if symbol.is_empty() {
        number
    } else {
        format!("{number} {symbol}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spacing has to be a number a person would choose, at every scale.
    /// A grid whose lines fall on 0.037 tells you nothing you could measure
    /// against.
    #[test]
    fn spacing_is_always_a_round_number() {
        for distance in [0.001, 0.05, 1.0, 7.3, 140.0, 4000.0, 1.0e6] {
            let spacing = nice_spacing(distance);
            let decade = 10f32.powf(spacing.log10().floor());
            let mantissa = spacing / decade;
            assert!(
                [1.0, 2.0, 5.0]
                    .iter()
                    .any(|s| (mantissa - s).abs() < 1e-3),
                "at {distance} the spacing is {spacing}, whose mantissa is {mantissa}"
            );
        }
    }

    /// And it has to keep the grid legible: somewhere around ten divisions
    /// across the view, never one and never a hundred.
    #[test]
    fn spacing_keeps_the_divisions_countable() {
        for distance in [0.01, 0.5, 1.0, 12.0, 140.0, 8000.0] {
            let divisions = distance / nice_spacing(distance);
            assert!(
                (2.0..=40.0).contains(&divisions),
                "at {distance} the view is {divisions} divisions across"
            );
        }
    }

    /// Monotone, so zooming out never makes the grid finer. A step function
    /// built from `floor` and a comparison is easy to get backwards, and the
    /// symptom — the grid flipping between two spacings as the camera moves —
    /// is subtle enough to live a long time.
    #[test]
    fn spacing_never_shrinks_as_the_camera_pulls_back() {
        let mut previous = 0.0f32;
        let mut distance = 0.001f32;
        while distance < 1.0e6 {
            let spacing = nice_spacing(distance);
            assert!(
                spacing >= previous,
                "at {distance} the spacing dropped from {previous} to {spacing}"
            );
            previous = spacing;
            distance *= 1.05;
        }
    }
}
