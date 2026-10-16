//! The axis legend of `docs/04-UX-SPEC.md` §2.2, bottom-left of the viewport.
//!
//! Two jobs, and the second is the one that earns it the screen space.
//!
//! It says which way the model is facing. In a path-traced viewport with no
//! wireframe and no world grid above the horizon, a symmetrical product gives
//! almost no orientation cue — a housing seen from the back looks exactly like
//! the same housing seen from the front until something moves.
//!
//! And it is a *control*: clicking an axis snaps the camera to that standard
//! view. The same seven views are on `Ctrl`+`Alt`+`1..7` per §6.1, but nobody
//! learns those on the first day, and a legend that only reports is a legend
//! that gets ignored after a week.
//!
//! # Why it is drawn rather than rendered
//!
//! It could be a tiny second render of a gizmo mesh. It is a painter instead,
//! because it must be readable at 80 pixels and legible over whatever the
//! render happens to put behind it — which means opaque discs, flat colours
//! and hinted text, none of which a path tracer produces. The projection is
//! orthographic on purpose too: a perspective axis gizmo foreshortens the axis
//! pointing at you into nothing, which is the one axis you most need to see.

use eframe::egui;
use tequila_core::math::{vec3, Vec3};
use tequila_render_core::camera::Camera;

use crate::orbit::StandardView;

/// Radius of the legend's active area, in points.
const RADIUS: f32 = 38.0;
/// Radius of a positive axis's disc.
const KNOB: f32 = 9.5;

/// One end of one axis.
struct Knob {
    /// Screen offset from the centre, already projected.
    offset: egui::Vec2,
    /// Towards the viewer is positive. Used only for draw order.
    depth: f32,
    color: egui::Color32,
    label: &'static str,
    positive: bool,
    view: StandardView,
}

/// Draw the legend and return the view the user clicked, if any.
///
/// Takes the whole viewport rect and places itself, rather than being given a
/// rect, so there is one definition of where it lives.
pub fn show(
    ui: &mut egui::Ui,
    viewport: egui::Rect,
    camera: &Camera,
    up: Vec3,
) -> Option<StandardView> {
    let centre = egui::pos2(
        viewport.left() + RADIUS + 18.0,
        viewport.bottom() - RADIUS - 18.0,
    );
    let area = egui::Rect::from_center_size(centre, egui::Vec2::splat(RADIUS * 2.0 + KNOB));

    // Camera basis. The forward vector is what the camera exposes; right and
    // up are recovered from it and the world up, which is exactly how the
    // camera was built in the first place.
    let forward = camera.forward().try_normalize().unwrap_or(vec3(0.0, 0.0, -1.0));
    let right = forward.cross(up).try_normalize().unwrap_or(Vec3::X);
    let screen_up = right.cross(forward).try_normalize().unwrap_or(Vec3::Y);

    // Orthographic: an axis's screen position is its projection onto the
    // camera's right and up, and its depth is its projection onto forward.
    let project = |direction: Vec3| -> (egui::Vec2, f32) {
        (
            egui::vec2(direction.dot(right), -direction.dot(screen_up)) * RADIUS,
            -direction.dot(forward),
        )
    };

    let axes = [
        (Vec3::X, "X", egui::Color32::from_rgb(232, 86, 86), StandardView::Right, StandardView::Left),
        (Vec3::Y, "Y", egui::Color32::from_rgb(126, 206, 108), StandardView::Top, StandardView::Bottom),
        (Vec3::Z, "Z", egui::Color32::from_rgb(88, 150, 246), StandardView::Front, StandardView::Back),
    ];

    let mut knobs = Vec::with_capacity(6);
    for (axis, label, color, positive_view, negative_view) in axes {
        for sign in [1.0f32, -1.0] {
            let (offset, depth) = project(axis * sign);
            knobs.push(Knob {
                offset,
                depth,
                color,
                label,
                positive: sign > 0.0,
                view: if sign > 0.0 { positive_view } else { negative_view },
            });
        }
    }
    // Painter's algorithm: the far ends first, so a near knob covers a far one
    // rather than the other way round. Without this the gizmo reads as
    // inside-out and gives the wrong orientation, which is worse than none.
    knobs.sort_by(|a, b| a.depth.total_cmp(&b.depth));

    let response = ui.interact(
        area,
        ui.id().with("axis-legend"),
        egui::Sense::click_and_drag(),
    );
    let pointer = response.hover_pos();

    // The centre is isometric.
    //
    // Every view gizmo has this, and it is the one view a person asks for by
    // name rather than by axis — "show me the three-quarter view". Leaving it
    // to a keyboard shortcut is what made it unreachable: `Ctrl`+`Alt`+`7` is
    // `AltGr`+`7` on a Windows keyboard, which many layouts eat before the
    // application sees it.
    let over_centre = pointer.is_some_and(|p| centre.distance(p) <= KNOB * 1.1);

    // Which knob the pointer is over — the nearest one under it, so an
    // overlapped pair resolves the way it looks.
    let hovered = (!over_centre).then(|| pointer).flatten().and_then(|p| {
        knobs
            .iter()
            .enumerate()
            .filter(|(_, k)| (centre + k.offset).distance(p) <= KNOB + 2.0)
            .max_by(|(_, a), (_, b)| a.depth.total_cmp(&b.depth))
            .map(|(i, _)| i)
    });

    let painter = ui.painter_at(viewport);
    // A backing disc, so the legend is readable over a white product as well
    // as over a dark one — faint at rest, firmer under the pointer, where it
    // says "this is a control" as Blender's navigation gizmo does.
    let near = ui.rect_contains_pointer(area);
    painter.circle_filled(
        centre,
        RADIUS + KNOB * 0.9,
        egui::Color32::from_black_alpha(if near { 95 } else { 45 }),
    );

    for (i, knob) in knobs.iter().enumerate() {
        let end = centre + knob.offset;
        let lit = hovered == Some(i);
        // The spoke is drawn only for the positive end. Six spokes is a
        // snowflake; three is an axis system.
        if knob.positive {
            painter.line_segment(
                [centre, end],
                egui::Stroke::new(2.0_f32, knob.color.gamma_multiply(0.85)),
            );
        }

        let fill = if knob.positive || lit {
            knob.color
        } else {
            // A hollow ring for the negative end: same colour, no fill. It
            // reads as "the other end of this axis" without needing a label.
            egui::Color32::from_black_alpha(150)
        };
        painter.circle(
            end,
            if lit { KNOB + 1.5 } else { KNOB },
            fill,
            egui::Stroke::new(if knob.positive { 0.0_f32 } else { 1.6 }, knob.color),
        );

        if knob.positive || lit {
            painter.text(
                end,
                egui::Align2::CENTER_CENTER,
                knob.label,
                egui::FontId::proportional(10.0),
                if knob.positive {
                    egui::Color32::from_rgb(18, 18, 18)
                } else {
                    egui::Color32::WHITE
                },
            );
        }
    }

    // Drawn last so it sits over the spokes that meet under it.
    painter.circle(
        centre,
        if over_centre { KNOB * 0.85 } else { KNOB * 0.62 },
        if over_centre {
            egui::Color32::from_rgb(235, 235, 235)
        } else {
            egui::Color32::from_rgb(120, 120, 120)
        },
        egui::Stroke::new(1.0_f32, egui::Color32::from_black_alpha(140)),
    );

    if over_centre {
        response
            .clone()
            .on_hover_text("Isometric view");
        if response.clicked() {
            return Some(StandardView::Isometric);
        }
    }

    if let Some(i) = hovered {
        let knob = &knobs[i];
        response
            .clone()
            .on_hover_text(format!("{} view", view_name(knob.view)));
        if response.clicked() {
            return Some(knob.view);
        }
    }
    None
}

/// True when the pointer is over the legend, so the viewport does not also
/// treat the click as a pick or the drag as an orbit.
pub fn contains(viewport: egui::Rect, at: egui::Pos2) -> bool {
    let centre = egui::pos2(
        viewport.left() + RADIUS + 18.0,
        viewport.bottom() - RADIUS - 18.0,
    );
    centre.distance(at) <= RADIUS + KNOB
}

fn view_name(view: StandardView) -> &'static str {
    match view {
        StandardView::Front => "Front",
        StandardView::Back => "Back",
        StandardView::Left => "Left",
        StandardView::Right => "Right",
        StandardView::Top => "Top",
        StandardView::Bottom => "Bottom",
        StandardView::Isometric => "Isometric",
    }
}
