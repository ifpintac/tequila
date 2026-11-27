//! Light outlines (`docs/04-UX-SPEC.md` §2.2): each light's shape and size
//! drawn over the view, in every shading, the Rendered view's included.
//!
//! A community member asked to "see the light geometry in the render
//! preview, at least an outline of the size, so we can see the size in
//! realtime". A light the camera does not see - most studio lights, and
//! every one the Lighting page's kit adds - had nothing in the view at all:
//! its size was a number on its page, and where its edge fell against the
//! product was a guess.
//!
//! An overlay, as the grid and the gizmo are, rather than geometry: it costs
//! the renderers nothing and never restarts them, it is never in a
//! reflection, a Screenshot or a render, and it follows a slider while it is
//! dragged, being drawn from the document every frame.
//!
//! What is drawn is what renders: an area light's edge from the same half
//! extents [`LightSettings::to_render_light`] gives the renderers, the
//! node's scale included; a bulb's sphere by its silhouette, as the camera
//! sees it; a spot's cone at its two angles. A point and a sun have no size,
//! and are marked at a fixed size on screen, the sun with the way its light
//! goes. A light the camera does not see (*Hide in final render*) is drawn
//! dashed, so the view says which lights the picture will show.

use eframe::egui;
use tequila_core::math::{Onb, Transform, Vec3};
use tequila_render_core::camera::{Camera, Projection};
use tequila_scene::lighting::{LightSettings, LightShape};
use tequila_scene::tree::{NodeId, NodeKind};

use crate::icons::{Icon, Tone};
use crate::preferences::Value;
use crate::project::Project;

/// The preference that shows them, kept with the other viewport toggles.
pub const SETTING: &str = "viewport.light_outlines";

/// Points round a circle or an ellipse.
const STEPS: usize = 64;

/// How long a sun's arrow is on screen, in points.
const SUN_POINTS: f32 = 60.0;

/// What a hover over the toolbar's button says.
const TIP: &str = "Each light's shape and size over the view, in every shading: a panel's edge, a bulb's sphere, a spot's cone. \
                   The selected light's is drawn brighter, with its size, and one hidden in the final render dashed. Never in screenshots or renders.";

/// Where the outlines are seen from: what a sphere's silhouette, and which
/// way a sun's arrowhead faces, depend on.
#[derive(Clone, Copy, Debug)]
pub struct Eye {
    pub position: Vec3,
    pub forward: Vec3,
    pub orthographic: bool,
}

impl Eye {
    pub fn of(camera: &Camera) -> Eye {
        Eye {
            position: camera.position(),
            forward: camera.forward(),
            orthographic: matches!(camera.projection, Projection::Orthographic { .. }),
        }
    }

    /// The way the eye looks at `point`.
    fn towards(&self, point: Vec3) -> Vec3 {
        if self.orthographic {
            self.forward
        } else {
            (point - self.position)
                .try_normalize()
                .unwrap_or(self.forward)
        }
    }
}

/// A light's outline, in the world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    /// Lines through points: an edge, a cone's rim, a stroke.
    pub lines: Vec<Vec<Vec3>>,
    /// Drawn fainter: a spot's inner cone, where its soft edge starts.
    pub faint: Vec<Vec<Vec3>>,
    /// Points marked at a fixed size on screen: a light's place, where it
    /// has no size of its own to draw.
    pub markers: Vec<Vec3>,
}

/// The outline of a light with `settings`, placed by `world`, seen from
/// `eye`. `reach` is how far a spot's cone and a sun's arrow run, which
/// have no length of their own.
pub fn outline(settings: &LightSettings, world: &Transform, eye: &Eye, reach: f32) -> Outline {
    let mut out = Outline::default();
    let centre = world.point(Vec3::ZERO);
    match settings.shape {
        LightShape::Area {
            width,
            height,
            disc,
            two_sided,
        } => {
            // The renderers' own half extents (`to_render_light`), so the
            // node's scale sizes the outline as it sizes the light.
            let u = world.vector(Vec3::X * (width * 0.5));
            let v = world.vector(Vec3::Y * (height * 0.5));
            out.lines.push(if disc {
                ring(centre, u, v)
            } else {
                vec![
                    centre - u - v,
                    centre + u - v,
                    centre + u + v,
                    centre - u + v,
                    centre - u - v,
                ]
            });
            // Which way it shines, along the cross of its width and height
            // as the renderers have it; both ways for a two-sided panel.
            if let Some(facing) = u.cross(v).try_normalize() {
                let tick = (u.length() + v.length()) * 0.35;
                out.lines.push(vec![centre, centre + facing * tick]);
                if two_sided {
                    out.lines.push(vec![centre, centre - facing * tick]);
                }
            }
        }
        LightShape::Point { radius } => {
            out.markers.push(centre);
            // Scaled as the renderers scale it.
            let radius = radius * world.vector(Vec3::X).length();
            if radius > 0.0 {
                if let Some((middle, normal, across)) = silhouette(centre, radius, eye) {
                    let onb = Onb::from_normal(normal);
                    out.lines.push(ring(middle, onb.t * across, onb.b * across));
                }
            }
        }
        LightShape::Spot {
            inner_degrees,
            outer_degrees,
            ..
        } => {
            out.markers.push(centre);
            let axis = world.vector(-Vec3::Z).try_normalize().unwrap_or(-Vec3::Z);
            let onb = Onb::from_normal(axis);
            // Clamped as the renderers clamp it: an inner angle past the
            // outer fills the cone.
            let outer = outer_degrees.clamp(0.0, 89.9).to_radians();
            let inner = inner_degrees.to_radians().clamp(0.0, outer);
            // Each rim `reach` from the lamp along the cone's side, so a
            // wide cone stays as long as a narrow one.
            let rim = |angle: f32| {
                let (along, across) = (reach * angle.cos(), reach * angle.sin());
                ring(centre + axis * along, onb.t * across, onb.b * across)
            };
            let edge = rim(outer);
            for k in 0..4 {
                out.lines.push(vec![centre, edge[k * STEPS / 4]]);
            }
            out.lines.push(edge);
            if inner > 1e-3 && outer - inner > 0.5f32.to_radians() {
                out.faint.push(rim(inner));
            }
        }
        LightShape::Sun { .. } => {
            // Where a sun is does not matter, only the way its light goes:
            // an arrow from its node that way.
            out.markers.push(centre);
            let way = world.vector(-Vec3::Z).try_normalize().unwrap_or(-Vec3::Z);
            let tip = centre + way * reach;
            out.lines.push(vec![centre, tip]);
            // Its head across the view, so it reads as a head.
            if let Some(side) = way.cross(eye.towards(tip)).try_normalize() {
                let back = tip - way * (reach * 0.18);
                out.lines.push(vec![
                    back + side * (reach * 0.1),
                    tip,
                    back - side * (reach * 0.1),
                ]);
            }
        }
    }
    out
}

/// Points round the ellipse at `centre` with half axes `u` and `v`, the
/// first point repeated at the end to close it.
fn ring(centre: Vec3, u: Vec3, v: Vec3) -> Vec<Vec3> {
    (0..=STEPS)
        .map(|i| {
            let a = i as f32 / STEPS as f32 * std::f32::consts::TAU;
            centre + u * a.cos() + v * a.sin()
        })
        .collect()
}

/// The circle a sphere's outline is from `eye`: its middle, the way it
/// faces and its radius. Seen in perspective, it is where the lines from
/// the eye touch the sphere, a little nearer the eye than the sphere's
/// middle and a little smaller than it; `None` from inside the sphere.
fn silhouette(centre: Vec3, radius: f32, eye: &Eye) -> Option<(Vec3, Vec3, f32)> {
    if eye.orthographic {
        return Some((centre, eye.forward, radius));
    }
    let to = centre - eye.position;
    let distance = to.length();
    if distance <= radius * 1.0001 {
        return None;
    }
    let normal = to * (1.0 / distance);
    let middle = centre - normal * (radius * radius / distance);
    let across = radius * (distance * distance - radius * radius).sqrt() / distance;
    Some((middle, normal, across))
}

/// How a light's outline is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub colour: egui::Color32,
    pub width: f32,
    /// Hidden in the final render: the camera does not see it.
    pub dashed: bool,
}

impl Look {
    /// A warm line, the accent colour and thicker when selected, faded when
    /// the light is off.
    pub fn of(settings: &LightSettings, selected: bool) -> Look {
        let colour = if selected {
            crate::theme::p().accent
        } else {
            egui::Color32::from_rgb(255, 206, 110)
        };
        Look {
            colour: if settings.enabled {
                colour
            } else {
                colour.gamma_multiply(0.45)
            },
            width: if selected { 1.8 } else { 1.2 },
            dashed: !settings.visibility.camera,
        }
    }
}

/// Whether `node` is selected, or is in a group that is.
fn selected(project: &Project, node: NodeId) -> bool {
    project.selection.contains(node)
        || project
            .document()
            .tree
            .ancestors(node)
            .into_iter()
            .any(|a| project.selection.contains(a))
}

/// Every light's outline, over the picture in `rect`, seen by `camera`
/// orbiting `target`.
pub fn draw(ui: &egui::Ui, rect: egui::Rect, camera: &Camera, target: Vec3, project: &Project) {
    // A 360° view has no straight lines to draw them with.
    if matches!(camera.projection, Projection::Equirectangular) {
        return;
    }
    let document = project.document();
    let painter = ui.painter_at(rect);
    let eye = Eye::of(camera);
    let aspect = rect.width().max(1.0) / rect.height().max(1.0);
    for node in document.tree.iter() {
        let NodeKind::Light { settings } = &node.kind else {
            continue;
        };
        // Hidden, a light is off in every engine, and has no outline.
        if !document.tree.is_effectively_visible(node.id) {
            continue;
        }
        let world = document.tree.world_transform(node.id);
        let at = world.point(Vec3::ZERO);
        let ahead = (at - eye.position).dot(eye.forward);
        let fixed = |points: f32| {
            crate::gizmo_ui::world_length_of_points(
                camera,
                aspect,
                rect.height(),
                ahead.max(1e-3),
                points,
            )
        };
        let reach = match settings.shape {
            // To the depth of what the view orbits, which is the product
            // the spot is usually aimed at.
            LightShape::Spot { .. } => {
                let to = (target - at).length();
                if to > 1e-4 {
                    to
                } else {
                    fixed(SUN_POINTS)
                }
            }
            LightShape::Sun { .. } => fixed(SUN_POINTS),
            LightShape::Area { .. } | LightShape::Point { .. } => 0.0,
        };
        let shape = outline(settings, &world, &eye, reach);
        let picked = selected(project, node.id);
        let look = Look::of(settings, picked);
        paint(&painter, camera, rect, &shape, look);
        // The selected light's size, in the scene's units, under its edge:
        // what its page's sliders are setting, read where it is.
        if picked {
            if let Some(text) = size_text(settings, &world, &document.units) {
                label(&painter, camera, rect, &shape, &text, look.colour);
            }
        }
    }
}

/// A light's size as its outline shows it, the node's scale included: a
/// panel's width by its height, a bulb's diameter, a spot's cone angle.
/// `None` for a point and a sun, which have none.
pub fn size_text(
    settings: &LightSettings,
    world: &Transform,
    units: &tequila_scene::units::SceneUnits,
) -> Option<String> {
    use tequila_scene::measure::format_length;
    match settings.shape {
        LightShape::Area { width, height, .. } => {
            let across = world.vector(Vec3::X * width).length();
            let up = world.vector(Vec3::Y * height).length();
            Some(format!(
                "{} × {}",
                format_length(across, units),
                format_length(up, units)
            ))
        }
        LightShape::Point { radius } if radius > 0.0 => Some(format!(
            "{} across",
            format_length(2.0 * radius * world.vector(Vec3::X).length(), units)
        )),
        LightShape::Spot { outer_degrees, .. } => Some(format!("{outer_degrees:.0}°")),
        LightShape::Point { .. } | LightShape::Sun { .. } => None,
    }
}

/// `text` on a small plate in `colour`, under the lowest point of `shape`
/// on screen; none for a light above the picture or beside it, where a
/// plate in its corner would name nothing in it.
fn label(
    painter: &egui::Painter,
    camera: &Camera,
    rect: egui::Rect,
    shape: &Outline,
    text: &str,
    colour: egui::Color32,
) {
    let Some(lowest) = plate_anchor(camera, rect, shape) else {
        return;
    };
    let galley = painter.layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(11.5),
        egui::Color32::BLACK,
    );
    let size = galley.size() + egui::vec2(10.0, 4.0);
    let at = egui::pos2(
        lowest
            .x
            .clamp(rect.left() + size.x * 0.5, rect.right() - size.x * 0.5),
        (lowest.y + 8.0 + size.y * 0.5).min(rect.bottom() - size.y * 0.5),
    );
    let plate = egui::Rect::from_center_size(at, size);
    painter.rect_filled(plate.expand(1.0), 4.0, egui::Color32::from_black_alpha(70));
    painter.rect_filled(plate, 3.0, colour);
    painter.galley(
        plate.min + egui::vec2(5.0, 2.0),
        galley,
        egui::Color32::BLACK,
    );
}

/// Where a light's size plate hangs: the lowest point of `shape` on screen,
/// or `None` when the light is above the picture or beside it. Below it, the
/// plate is held at the picture's foot.
fn plate_anchor(camera: &Camera, rect: egui::Rect, shape: &Outline) -> Option<egui::Pos2> {
    let lowest = shape
        .lines
        .iter()
        .flatten()
        .chain(&shape.markers)
        .filter(|p| in_front(camera, **p))
        .filter_map(|p| crate::gizmo_ui::project_point(camera, rect, *p))
        .filter(|p| p.x.is_finite() && p.y.is_finite())
        .max_by(|a, b| a.y.total_cmp(&b.y))?;
    (lowest.y >= rect.top() && lowest.x >= rect.left() && lowest.x <= rect.right())
        .then_some(lowest)
}

/// Paint `shape` as `look` says: each line on a thin dark edge, so it reads
/// over a white studio and a glowing panel as well as a black one.
fn paint(painter: &egui::Painter, camera: &Camera, rect: egui::Rect, shape: &Outline, look: Look) {
    let edge = |width: f32| egui::Stroke::new(width + 2.0, egui::Color32::from_black_alpha(70));
    let line = |points: &[Vec3], stroke: egui::Stroke| {
        for run in on_screen(points, camera, rect) {
            if look.dashed {
                painter.extend(egui::Shape::dashed_line(&run, edge(stroke.width), 6.0, 4.0));
                painter.extend(egui::Shape::dashed_line(&run, stroke, 6.0, 4.0));
            } else {
                painter.add(egui::Shape::line(run.clone(), edge(stroke.width)));
                painter.add(egui::Shape::line(run, stroke));
            }
        }
    };
    for points in &shape.lines {
        line(points, egui::Stroke::new(look.width, look.colour));
    }
    for points in &shape.faint {
        line(
            points,
            egui::Stroke::new(
                (look.width - 0.4).max(0.8),
                look.colour.gamma_multiply(0.55),
            ),
        );
    }
    for &point in &shape.markers {
        if let Some(at) = in_front(camera, point)
            .then(|| crate::gizmo_ui::project_point(camera, rect, point))
            .flatten()
        {
            painter.circle_stroke(at, 4.5, edge(look.width));
            painter.circle_stroke(at, 4.5, egui::Stroke::new(look.width, look.colour));
            painter.circle_filled(at, 1.6, look.colour);
        }
    }
}

/// How near the camera a line may come before it is cut: a point at or
/// behind the eye has no place on screen, and a line to one, drawn to where
/// it projects, flies across the view.
const NEAR: f32 = 1e-3;

fn depth(camera: &Camera, point: Vec3) -> f32 {
    (point - camera.position()).dot(camera.forward())
}

fn in_front(camera: &Camera, point: Vec3) -> bool {
    depth(camera, point) > NEAR
}

/// The line through `points` on screen, cut where it passes behind the
/// camera: the runs of it in front.
fn on_screen(points: &[Vec3], camera: &Camera, rect: egui::Rect) -> Vec<Vec<egui::Pos2>> {
    let mut runs = Vec::new();
    let mut run: Vec<egui::Pos2> = Vec::new();
    // The run so far kept, if it is a line, and a new one begun.
    let finish = |runs: &mut Vec<Vec<egui::Pos2>>, run: &mut Vec<egui::Pos2>| {
        let done = std::mem::take(run);
        if done.len() > 1 {
            runs.push(done);
        }
    };
    let to_screen = |p: Vec3| crate::gizmo_ui::project_point(camera, rect, p);
    for pair in points.windows(2) {
        let (mut a, mut b) = (pair[0], pair[1]);
        let (da, db) = (depth(camera, a), depth(camera, b));
        let (cut_a, cut_b) = (da <= NEAR, db <= NEAR);
        if cut_a && cut_b {
            finish(&mut runs, &mut run);
            continue;
        }
        // Each end behind the camera brought forward along the line to
        // just in front of it.
        if cut_a {
            a = a + (b - a) * ((NEAR * 2.0 - da) / (db - da));
        }
        if cut_b {
            b = b + (a - b) * ((NEAR * 2.0 - db) / (depth(camera, a) - db));
        }
        let (Some(sa), Some(sb)) = (to_screen(a), to_screen(b)) else {
            finish(&mut runs, &mut run);
            continue;
        };
        if cut_a || run.is_empty() {
            finish(&mut runs, &mut run);
            run.push(sa);
        }
        run.push(sb);
        if cut_b {
            finish(&mut runs, &mut run);
        }
    }
    finish(&mut runs, &mut run);
    runs
}

impl crate::TequilaApp {
    /// The outlines over the picture in `rect`, when they are shown.
    pub(crate) fn draw_light_outlines(&self, ui: &egui::Ui, rect: egui::Rect) {
        if self.prefs.show_light_outlines {
            draw(
                ui,
                rect,
                &self.camera.camera,
                self.camera.target,
                &self.project,
            );
        }
    }

    /// The viewport toolbar's button, at `at`: lit while the outlines show,
    /// and a click shows or hides them, kept for the next session as the
    /// composition guides beside it are.
    pub(crate) fn light_outlines_button(&mut self, ui: &mut egui::Ui, at: egui::Rect) {
        let on = self.prefs.show_light_outlines;
        let response = ui.interact(
            at,
            ui.id().with(("viewport-tool", "light-outlines")),
            egui::Sense::click_and_drag(),
        );
        crate::icons::paint_button(
            ui,
            at,
            &response,
            Icon::Light,
            if on { Tone::On } else { Tone::Quiet },
            true,
        );
        if crate::icons::tip(
            response,
            if on {
                "Light outlines: shown"
            } else {
                "Light outlines: hidden"
            },
            TIP,
            "",
        )
        .clicked()
        {
            self.show_light_outlines(!on);
        }
    }

    /// View › Light outlines.
    pub(crate) fn light_outlines_menu_item(&mut self, ui: &mut egui::Ui) {
        let mut on = self.prefs.show_light_outlines;
        if ui
            .add(crate::field::Checkbox::new(&mut on, "Light outlines"))
            .on_hover_text(TIP)
            .changed()
        {
            self.show_light_outlines(on);
        }
    }

    /// Show or hide the outlines, and keep the choice.
    fn show_light_outlines(&mut self, on: bool) {
        if let Err(error) = self.set_preference(SETTING, Value::Bool(on)) {
            self.toasts
                .warn(format!("The light outlines could not be kept: {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec3, Mat4};
    use tequila_render_core::light::LightKind;

    fn light(shape: LightShape) -> LightSettings {
        LightSettings {
            shape,
            ..LightSettings::default()
        }
    }

    fn eye_at(position: Vec3) -> Eye {
        Eye {
            position,
            forward: (-position).try_normalize().unwrap_or(-Vec3::Z),
            orthographic: false,
        }
    }

    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    /// A turned, moved and scaled placement, as a light in a studio's group
    /// can have.
    fn placed() -> Transform {
        Transform::from_matrix(
            Mat4::translation(vec3(0.3, 1.2, -0.4))
                * Mat4::rotation(vec3(1.0, 0.4, 0.2).normalize(), 0.9)
                * Mat4::scale(vec3(2.0, 0.5, 1.5)),
        )
        .expect("invertible")
    }

    /// A panel's outline is its four corners where the renderers put its
    /// edge, with the node's scale, at any size; with a stroke out of its
    /// middle the way it shines, and one each way when it shines both ways.
    #[test]
    fn a_panels_outline_is_its_edge_where_it_renders_at_any_size() {
        let world = placed();
        let eye = eye_at(vec3(0.0, 1.0, 5.0));
        for (width, height) in [(1.0, 1.0), (0.3, 1.8), (40.0, 2.5)] {
            for two_sided in [false, true] {
                let settings = light(LightShape::Area {
                    width,
                    height,
                    disc: false,
                    two_sided,
                });
                let shape = outline(&settings, &world, &eye, 0.0);
                let Some(LightKind::Area(area)) = settings.to_render_light(&world).map(|l| l.kind)
                else {
                    panic!("an area light")
                };
                let corners = [
                    area.position - area.half_u - area.half_v,
                    area.position + area.half_u - area.half_v,
                    area.position + area.half_u + area.half_v,
                    area.position - area.half_u + area.half_v,
                ];
                let edge = &shape.lines[0];
                assert_eq!(edge.len(), 5, "four corners and back");
                for (drawn, rendered) in edge.iter().zip(corners.iter().chain([&corners[0]])) {
                    assert!(
                        near(*drawn, *rendered),
                        "{width} x {height}: {drawn:?} drawn, {rendered:?} rendered"
                    );
                }
                let facing = area.half_u.cross(area.half_v).normalize();
                let ticks: Vec<Vec3> = shape.lines[1..]
                    .iter()
                    .map(|l| (l[1] - l[0]).normalize())
                    .collect();
                assert_eq!(ticks.len(), if two_sided { 2 } else { 1 });
                assert!(
                    near(ticks[0], facing),
                    "it shines along {facing:?}, the stroke says {:?}",
                    ticks[0]
                );
                if two_sided {
                    assert!(near(ticks[1], -facing));
                }
                assert!(shape.markers.is_empty() && shape.faint.is_empty());
            }
        }
    }

    /// A disc is the ellipse inside its width and height, touching each
    /// side's middle, and grows with them.
    #[test]
    fn a_discs_outline_is_the_ellipse_in_its_width_and_height() {
        let world = placed();
        let eye = eye_at(vec3(0.0, 1.0, 5.0));
        for (width, height) in [(1.0, 1.0), (2.0, 0.5), (0.05, 0.08)] {
            let settings = light(LightShape::Area {
                width,
                height,
                disc: true,
                two_sided: false,
            });
            let shape = outline(&settings, &world, &eye, 0.0);
            let Some(LightKind::Area(area)) = settings.to_render_light(&world).map(|l| l.kind)
            else {
                panic!("an area light")
            };
            let edge = &shape.lines[0];
            assert!(near(edge[0], *edge.last().unwrap()), "closed");
            for point in edge {
                // In the panel's own half extents, every point is on the
                // unit circle.
                let d = *point - area.position;
                let (x, y) = (
                    d.dot(area.half_u) / area.half_u.dot(area.half_u),
                    d.dot(area.half_v) / area.half_v.dot(area.half_v),
                );
                assert!(
                    (x * x + y * y - 1.0).abs() < 1e-3,
                    "{width} x {height}: {point:?} is off the ellipse ({x}, {y})"
                );
            }
            for middle in [area.position + area.half_u, area.position - area.half_v] {
                assert!(
                    edge.iter().any(|p| near(*p, middle)),
                    "it does not touch {middle:?}"
                );
            }
        }
    }

    /// A bulb is outlined by its silhouette: a circle every point of which
    /// is on the sphere, where the line from the eye only touches it, at
    /// its radius scaled by the node. A true point is a marker alone.
    #[test]
    fn a_bulbs_outline_is_its_silhouette_and_a_point_a_marker() {
        let world = Transform::from_matrix(
            Mat4::translation(vec3(0.5, 1.0, 0.0)) * Mat4::scale(Vec3::splat(2.0)),
        )
        .expect("invertible");
        for position in [vec3(0.0, 1.0, 4.0), vec3(3.0, 2.5, 0.4)] {
            let eye = eye_at(position);
            for radius in [0.01, 0.2, 0.9] {
                let shape = outline(&light(LightShape::Point { radius }), &world, &eye, 0.0);
                let centre = world.point(Vec3::ZERO);
                assert_eq!(shape.markers, vec![centre]);
                let rendered = match light(LightShape::Point { radius })
                    .to_render_light(&world)
                    .map(|l| l.kind)
                {
                    Some(LightKind::Point(point)) => point.radius,
                    _ => panic!("a point light"),
                };
                assert!((rendered - radius * 2.0).abs() < 1e-5);
                for point in &shape.lines[0] {
                    assert!(
                        ((*point - centre).length() - rendered).abs() < 1e-3 * rendered.max(1.0),
                        "radius {radius}: off the sphere"
                    );
                    let sight = (*point - position).normalize();
                    let normal = (*point - centre).normalize();
                    assert!(
                        sight.dot(normal).abs() < 1e-3,
                        "radius {radius}: the eye's line is not a tangent there"
                    );
                }
            }
        }
        let point = outline(
            &light(LightShape::Point { radius: 0.0 }),
            &world,
            &eye_at(vec3(0.0, 1.0, 4.0)),
            0.0,
        );
        assert!(
            point.lines.is_empty() && point.markers.len() == 1,
            "{point:?}"
        );
        // From inside the bulb there is no outline to see, and nothing fails.
        let inside = outline(
            &light(LightShape::Point { radius: 1.0 }),
            &world,
            &eye_at(world.point(Vec3::ZERO)),
            0.0,
        );
        assert!(inside.lines.is_empty());
    }

    /// A spot's cone runs from the lamp the way it aims, its rim at the
    /// cone angle and its soft edge's start at the inner one, each `reach`
    /// along the side; a wider cone is wider, not longer.
    #[test]
    fn a_spots_outline_is_its_cone_at_its_angles() {
        let world = placed();
        let eye = eye_at(vec3(0.0, 1.0, 5.0));
        for (inner, outer, reach) in [(10.0, 20.0, 1.0), (0.0, 45.0, 2.5), (60.0, 89.0, 0.4)] {
            let settings = light(LightShape::Spot {
                inner_degrees: inner,
                outer_degrees: outer,
                falloff: 1.0,
            });
            let shape = outline(&settings, &world, &eye, reach);
            let Some(LightKind::Spot(spot)) = settings.to_render_light(&world).map(|l| l.kind)
            else {
                panic!("a spot")
            };
            assert_eq!(shape.markers, vec![spot.position]);
            let angle_of = |p: Vec3| {
                (p - spot.position)
                    .normalize()
                    .dot(spot.direction)
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees()
            };
            let rim = shape.lines.last().unwrap();
            for point in rim {
                assert!(
                    (angle_of(*point) - outer).abs() < 0.01,
                    "{outer}°: a rim point at {}°",
                    angle_of(*point)
                );
                assert!(((*point - spot.position).length() - reach).abs() < 1e-4);
            }
            assert_eq!(shape.lines.len(), 5, "four sides and the rim");
            if inner > 0.0 {
                for point in &shape.faint[0] {
                    assert!(
                        (angle_of(*point) - inner).abs() < 0.01,
                        "{inner}°: an inner point at {}°",
                        angle_of(*point)
                    );
                }
            } else {
                assert!(
                    shape.faint.is_empty(),
                    "a cone with no soft edge has no inner rim"
                );
            }
        }
        // An inner angle past the outer fills the cone, as it renders.
        let filled = outline(
            &light(LightShape::Spot {
                inner_degrees: 50.0,
                outer_degrees: 30.0,
                falloff: 1.0,
            }),
            &world,
            &eye,
            1.0,
        );
        assert!(filled.faint.is_empty());
    }

    /// A sun is an arrow from its node the way its light goes, `reach`
    /// long, with a head.
    #[test]
    fn a_suns_outline_is_an_arrow_the_way_it_shines() {
        let world = placed();
        let settings = light(LightShape::Sun {
            angle_degrees: 0.53,
        });
        let shape = outline(&settings, &world, &eye_at(vec3(0.0, 1.0, 5.0)), 0.7);
        let Some(LightKind::Distant(sun)) = settings.to_render_light(&world).map(|l| l.kind) else {
            panic!("a sun")
        };
        let shaft = &shape.lines[0];
        assert!(near(shaft[0], world.point(Vec3::ZERO)));
        assert!(
            near((shaft[1] - shaft[0]) * (1.0 / 0.7), sun.direction),
            "it points along {:?}",
            (shaft[1] - shaft[0]).normalize()
        );
        assert_eq!(shape.lines[1].len(), 3, "a head");
    }

    /// The outline follows the light: changing its size on its page, as a
    /// slider does every frame of a drag, moves the drawn edge with it.
    #[test]
    fn the_outline_follows_a_change_of_size() {
        let eye = eye_at(vec3(0.0, 1.0, 5.0));
        let small = outline(
            &light(LightShape::Area {
                width: 0.5,
                height: 0.5,
                disc: false,
                two_sided: false,
            }),
            &Transform::IDENTITY,
            &eye,
            0.0,
        );
        let large = outline(
            &light(LightShape::Area {
                width: 2.0,
                height: 0.5,
                disc: false,
                two_sided: false,
            }),
            &Transform::IDENTITY,
            &eye,
            0.0,
        );
        assert!(
            near(small.lines[0][2], vec3(0.25, 0.25, 0.0))
                && near(large.lines[0][2], vec3(1.0, 0.25, 0.0))
        );
    }

    /// A line that runs behind the camera is cut where it passes it, not
    /// drawn to where a point behind the eye would land.
    #[test]
    fn a_line_behind_the_camera_is_cut_where_it_passes_it() {
        let camera = Camera::look_at(vec3(0.0, 0.0, 2.0), Vec3::ZERO, Vec3::Y);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 300.0));
        let across = vec![vec3(-0.5, 0.0, 0.0), vec3(0.5, 0.0, 0.0)];
        assert_eq!(on_screen(&across, &camera, rect).len(), 1);
        let behind = vec![vec3(0.0, 0.0, 3.0), vec3(0.0, 0.5, 4.0)];
        assert!(on_screen(&behind, &camera, rect).is_empty());
        // In front, behind, and in front again: two runs, each in front.
        let through = vec![
            vec3(-0.5, 0.0, 0.0),
            vec3(0.0, 0.0, 4.0),
            vec3(0.5, 0.0, 0.0),
        ];
        let runs = on_screen(&through, &camera, rect);
        assert_eq!(runs.len(), 2, "{runs:?}");
        for run in runs {
            assert!(run.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
        }
    }

    /// The look: dashed when the camera does not see the light, faded when
    /// it is off, brighter and thicker when it is selected.
    #[test]
    fn a_hidden_light_is_dashed_an_off_one_faded_and_the_selected_one_bright() {
        let shown = LightSettings::default();
        let hidden = LightSettings {
            visibility: tequila_render_core::light::Visibility::HIDDEN,
            ..LightSettings::default()
        };
        let off = LightSettings {
            enabled: false,
            ..LightSettings::default()
        };
        assert!(!Look::of(&shown, false).dashed && Look::of(&hidden, false).dashed);
        assert!(Look::of(&off, false).colour.a() < Look::of(&shown, false).colour.a());
        assert!(Look::of(&shown, true).width > Look::of(&shown, false).width);
    }

    /// Every light in a scene is drawn, in a panel, a bulb, a spot and a sun,
    /// over the view; hidden, a light has none; and the toolbar's button
    /// hides them all, kept for the next session.
    #[test]
    fn every_shown_light_is_drawn_and_the_button_hides_them_and_is_kept() {
        let folder =
            std::env::temp_dir().join(format!("tequila-light-outlines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let file = folder.join("preferences.txt");
        let mut app = crate::TequilaApp::for_tests();
        app.load_preferences(file.clone());
        assert!(app.prefs.show_light_outlines, "shown as Tequila comes");
        let shapes = [
            LightShape::Area {
                width: 0.4,
                height: 0.3,
                disc: false,
                two_sided: false,
            },
            LightShape::Point { radius: 0.05 },
            LightShape::Spot {
                inner_degrees: 15.0,
                outer_degrees: 30.0,
                falloff: 1.0,
            },
            LightShape::Sun {
                angle_degrees: 0.53,
            },
        ];
        for (i, shape) in shapes.into_iter().enumerate() {
            app.project
                .apply(tequila_scene::Command::AddLight {
                    parent: None,
                    name: format!("Light {i}"),
                    settings: Box::new(light(shape)),
                })
                .expect("added");
        }
        let lights: Vec<NodeId> = app
            .project
            .document()
            .tree
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Light { .. }))
            .map(|n| n.id)
            .collect();
        assert_eq!(lights.len(), 4);
        for (i, &node) in lights.iter().enumerate() {
            let at = vec3(-0.6 + 0.4 * i as f32, 0.4, 0.3);
            app.project
                .apply(tequila_scene::Command::set_transform(
                    node,
                    Transform::translation(at),
                ))
                .expect("placed");
        }
        app.camera.camera = Camera::look_at(vec3(0.0, 0.5, 3.0), vec3(0.0, 0.2, 0.0), Vec3::Y);
        app.camera.target = vec3(0.0, 0.2, 0.0);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 400.0));
        let painted = |app: &crate::TequilaApp| {
            let ctx = egui::Context::default();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.draw_light_outlines(ui, rect));
                },
            );
            output.shapes.len()
        };
        let all = painted(&app);
        assert!(all > 0, "nothing was drawn");
        app.project
            .apply(tequila_scene::Command::set_visibility(lights[0], false))
            .expect("hidden");
        let fewer = painted(&app);
        assert!(
            fewer < all,
            "a hidden light was still outlined: {fewer} shapes, {all} with it"
        );

        app.show_light_outlines(false);
        assert_eq!(
            painted(&app),
            painted_nothing(rect),
            "the button did not hide them"
        );
        let kept = std::fs::read_to_string(&file).expect("kept");
        assert!(kept.contains(&format!("{SETTING}=off\n")), "{kept}");
        let mut next = crate::TequilaApp::for_tests();
        next.load_preferences(file);
        assert!(
            !next.prefs.show_light_outlines,
            "the next session showed them"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A script shows and hides the outlines for the session, as for a
    /// picture of the view, and reads back whether they show; the person's
    /// own choice, kept in the preferences file, is left as it was.
    #[test]
    fn a_script_shows_and_hides_the_outlines_for_the_session() {
        use tequila_format::json::Value;
        let mut app = crate::TequilaApp::for_tests();
        let mut ask = |line: &str| {
            let request = tequila_scripting::protocol::parse_request(line).expect("a request");
            match crate::channel::handle(&mut app, &request) {
                tequila_scripting::protocol::Reply::Value(value) => value,
                other => panic!("{line}: {other:?}"),
            }
        };
        let shown = |state: &Value| state.get("light_outlines").and_then(Value::as_bool);
        assert_eq!(shown(&ask(r#"{"op":"ui"}"#)), Some(true));
        assert_eq!(
            shown(&ask(r#"{"op":"ui","light_outlines":false}"#)),
            Some(false)
        );
        assert_eq!(
            shown(&ask(r#"{"op":"ui","light_outlines":true}"#)),
            Some(true)
        );
    }

    /// The selected light's size is written as its page and the renderers
    /// have it, the node's scale included, in the scene's units.
    #[test]
    fn the_size_written_is_the_size_that_renders() {
        let units = tequila_scene::units::SceneUnits::default();
        let scaled = Transform::from_matrix(Mat4::scale(vec3(2.0, 0.5, 1.0))).expect("invertible");
        let panel = light(LightShape::Area {
            width: 0.3,
            height: 0.8,
            disc: false,
            two_sided: false,
        });
        let text = size_text(&panel, &scaled, &units).expect("a panel has a size");
        let (across, up) = (
            tequila_scene::measure::format_length(0.6, &units),
            tequila_scene::measure::format_length(0.4, &units),
        );
        assert_eq!(text, format!("{across} × {up}"));
        let bulb = size_text(
            &light(LightShape::Point { radius: 0.05 }),
            &Transform::IDENTITY,
            &units,
        )
        .expect("a bulb has a size");
        assert_eq!(
            bulb,
            format!(
                "{} across",
                tequila_scene::measure::format_length(0.1, &units)
            )
        );
        assert_eq!(
            size_text(
                &light(LightShape::Spot {
                    inner_degrees: 10.0,
                    outer_degrees: 35.0,
                    falloff: 1.0
                }),
                &Transform::IDENTITY,
                &units
            )
            .as_deref(),
            Some("35°")
        );
        assert_eq!(
            size_text(
                &light(LightShape::Point { radius: 0.0 }),
                &Transform::IDENTITY,
                &units
            ),
            None
        );
        assert_eq!(
            size_text(
                &light(LightShape::Sun { angle_degrees: 0.5 }),
                &Transform::IDENTITY,
                &units
            ),
            None
        );
    }

    /// The size plate hangs under a light in the picture, and is not drawn
    /// for one above the picture or off to its side, where it would sit in
    /// a corner naming nothing there.
    #[test]
    fn the_size_plate_hangs_under_a_light_in_the_picture_only() {
        let camera = Camera::look_at(vec3(0.0, 0.0, 3.0), Vec3::ZERO, Vec3::Y);
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(640.0, 400.0));
        let eye = Eye::of(&camera);
        let panel = light(LightShape::Area {
            width: 0.4,
            height: 0.3,
            disc: false,
            two_sided: false,
        });
        let at = |position: Vec3| {
            plate_anchor(
                &camera,
                rect,
                &outline(&panel, &Transform::translation(position), &eye, 0.0),
            )
        };
        let below = at(Vec3::ZERO).expect("a light in the middle has a plate");
        let bottom_edge =
            crate::gizmo_ui::project_point(&camera, rect, vec3(0.0, -0.15, 0.0)).expect("in front");
        assert!(
            (below.y - bottom_edge.y).abs() < 1.0,
            "the plate hangs from {below:?}, not the panel's foot {bottom_edge:?}"
        );
        assert!(
            at(vec3(0.0, 3.0, 0.0)).is_none(),
            "a light above the picture has a plate"
        );
        assert!(
            at(vec3(6.0, 0.0, 0.0)).is_none(),
            "a light beside the picture has a plate"
        );
        assert!(
            at(vec3(0.0, 0.0, 4.0)).is_none(),
            "a light behind the camera has a plate"
        );
    }

    /// What a frame paints with nothing drawn into it.
    fn painted_nothing(rect: egui::Rect) -> usize {
        let ctx = egui::Context::default();
        ctx.run(
            egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |_| {});
            },
        )
        .shapes
        .len()
    }
}
