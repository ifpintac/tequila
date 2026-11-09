//! Texture mapping you can see: a texture laid on flat, around a cylinder,
//! around a sphere or from three sides, drawn on the part as the shape it
//! is projected from, with one repeat of it outlined, and handles to move,
//! size and turn it in the viewport (`docs/04` §3.3).
//!
//! Armed from the Place button beside a texture's mapping in the Material
//! tab, as a label's placement is: while armed the primary button is the
//! texture's - a drag on the part slides it, the corner square sizes it
//! about the repeat's middle, the circle turns it - and Ctrl and Shift with
//! the wheel size and turn it too. Escape, or the button again, ends it.
//! The other buttons still look around. Every change is the Mapping panel's
//! own command, and a drag is one step to undo.

use crate::gizmo_ui;
use crate::orchestrator::Orchestrator;
use crate::project::Project;
use eframe::egui;
use tequila_core::math::{Aabb, Transform, Vec3};
use tequila_materials::material_type::Param;
use tequila_render_core::camera::Camera;
use tequila_scene::tree::{MaterialKey, NodeId};
use tequila_scene::{decal_fit, Command};
use tequila_texture::projection::{Mapping, Projection, SurfacePoint};

/// Which texture is being placed: a material's parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub material: MaterialKey,
    pub param: Param,
}

#[derive(Default)]
pub struct MappingGizmo {
    target: Option<Target>,
    drag: Option<Drag>,
}

#[derive(Clone, Copy, Debug)]
enum Grab {
    /// The texture coordinates under the cursor when the drag began.
    Move { at: (f32, f32) },
    /// The cursor's distance from the repeat's middle when it began.
    Size { from: f32 },
    /// Its angle about the middle when it began.
    Turn { from: f32 },
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    grab: Grab,
    start: Mapping,
    /// The anchor on screen when the drag began: what a size and a turn are
    /// measured about.
    centre: egui::Pos2,
    /// Its texture coordinates, which a size and a turn keep in place.
    anchor: (f32, f32),
}

/// How near a handle, in points, a press takes it.
const REACH: f32 = 10.0;

/// The texture, the part it is placed on, and where the camera is in the
/// part's own space.
struct Placed {
    mapping: Mapping,
    node: NodeId,
    bounds: Aabb,
    world: Transform,
    viewer: Vec3,
}

impl Placed {
    /// Where the handles are anchored: the texture coordinates where the
    /// projection faces the camera through the part
    /// (`decal_fit::facing_coords`), read through `mapping`.
    fn anchor(&self, mapping: &Mapping) -> Option<(f32, f32)> {
        decal_fit::facing_coords(mapping, self.bounds, self.viewer)
    }
}

impl MappingGizmo {
    pub fn armed(&self) -> Option<Target> {
        self.target
    }

    pub fn disarm(&mut self) {
        self.target = None;
        self.drag = None;
    }

    /// Arm for `target`, or disarm if it was already.
    pub fn toggle(&mut self, target: Target) {
        if self.target == Some(target) {
            self.disarm();
        } else {
            self.target = Some(target);
            self.drag = None;
        }
    }

    /// Whether a scroll with these modifiers is the texture's rather than
    /// the camera's.
    pub fn takes_scroll(&self, modifiers: egui::Modifiers) -> bool {
        self.target.is_some() && (modifiers.command || modifiers.shift)
    }

    fn placed(&self, project: &Project, camera: &Camera) -> Option<(Target, Placed)> {
        let target = self.target?;
        let document = project.document();
        let graph = &document.materials.get(target.material)?.graph;
        let mapping = tequila_scene::command::mapping_of(graph, target.param)?;
        if mapping.projection == Projection::Uv {
            return None;
        }
        let (node, bounds) = decal_fit::part_of(document, target.material)?;
        let world = document.tree.world_transform(node);
        let viewer = world.inv_point(camera.position());
        Some((target, Placed { mapping, node, bounds, world, viewer }))
    }

    /// Texture coordinates `(u, v)` on screen.
    fn on_screen(placed: &Placed, mapping: &Mapping, camera: &Camera, rect: egui::Rect, u: f32, v: f32) -> Option<egui::Pos2> {
        let p = decal_fit::frame_point(mapping, placed.bounds, Some(placed.viewer), u, v)?;
        gizmo_ui::project_point(camera, rect, placed.world.point(p))
    }

    /// The anchor - where the projection faces the camera through the part
    /// - with the size square toward a repeat's corner from it and the
    /// turning circle toward its top, each between 36 and 120 points out: a
    /// size is the ratio of distances from the anchor and a turn the angle
    /// about it, so they work at any reach, and a repeat bigger than the
    /// view (a metre of texture on a 20 cm part) keeps its handles on the
    /// part in sight.
    fn handles(placed: &Placed, mapping: &Mapping, camera: &Camera, rect: egui::Rect) -> Option<(egui::Pos2, egui::Pos2, egui::Pos2)> {
        let (u, v) = placed.anchor(mapping)?;
        let centre = Self::on_screen(placed, mapping, camera, rect, u, v)?;
        let toward = |du: f32, dv: f32, fallback: egui::Vec2, more: f32| {
            let to = Self::on_screen(placed, mapping, camera, rect, u + du, v + dv).map(|p| p - centre).filter(|d| d.length() > 1e-3 && d.is_finite()).unwrap_or(fallback);
            centre + to.normalized() * (to.length() + more).clamp(36.0, 120.0)
        };
        let mut corner = toward(0.5, 0.5, egui::vec2(60.0, -60.0), 0.0);
        // Beyond the top, so it is not under the square.
        let turn = toward(0.0, 0.5, egui::vec2(0.0, -80.0), 22.0);
        // At least an eighth of a turn apart round the anchor: on a wrap
        // half a repeat round and half a repeat up can both point up.
        let (a, b) = (corner - centre, turn - centre);
        let apart = (a.angle() - b.angle() + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        if apart.abs() < 0.75 {
            let side = if apart >= 0.0 { 0.8 } else { -0.8 };
            corner = centre + egui::Vec2::angled(b.angle() + side) * a.length().max(48.0);
        }
        Some((centre, corner, turn))
    }

    /// The texture coordinates under `at` on the part, read through
    /// `mapping`: from three sides, the projection that counts most there.
    fn texture_at(placed: &Placed, mapping: &Mapping, project: &Project, orchestrator: &Orchestrator, camera: &Camera, rect: egui::Rect, at: egui::Pos2) -> Option<(f32, f32)> {
        let ray = gizmo_ui::ray_through(camera, rect, at)?;
        let hit = orchestrator.pick(&ray)?;
        if project.mapping().node_of(hit.instance) != Some(placed.node) {
            return None;
        }
        let point = SurfacePoint {
            position: placed.world.inv_point(hit.position),
            normal: placed.world.inv_normal(hit.shading_normal).try_normalize().unwrap_or(Vec3::Y),
            uv: tequila_core::math::Vec2::ZERO,
            footprint: 0.0,
            scale: 1.0,
            uv_footprint: None,
            curvature: 0.0,
            occlusion: 0.0,
            vertex_color: tequila_core::spectrum::Spectrum::ONE,
            world: Default::default(),
        };
        mapping
            .project(&point)
            .into_iter()
            .max_by(|a, b| a.weight.total_cmp(&b.weight))
            .map(|c| (c.coords.u, c.coords.v))
    }

    /// Take the primary button while armed. Returns whether it did, so the
    /// viewport neither selects nor orbits with it.
    pub fn interact(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        camera: &Camera,
        project: &mut Project,
        orchestrator: &Orchestrator,
        response: &egui::Response,
    ) -> bool {
        if self.target.is_none() {
            return false;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.disarm();
            project.status = "Texture placement finished".into();
            return true;
        }
        // The texture may have been taken off, or set back to UV, while the
        // mode was armed.
        let Some((target, placed)) = self.placed(project, camera) else {
            self.disarm();
            return false;
        };
        if response.drag_stopped() {
            self.drag = None;
            project.end_interaction();
        }
        if response.hovered() {
            let (scroll, modifiers) = ui.input(|i| (i.raw_scroll_delta.y, i.modifiers));
            if scroll != 0.0 && (modifiers.command || modifiers.shift) {
                let steps = scroll / 120.0;
                let anchor = placed.anchor(&placed.mapping).unwrap_or((0.5, 0.5));
                let next = if modifiers.shift {
                    turned(&placed.mapping, steps * 5.0f32.to_radians(), anchor)
                } else {
                    sized(&placed.mapping, (steps * 0.08).exp(), anchor)
                };
                apply(project, target, next);
                return true;
            }
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            let press = ui.input(|i| i.pointer.press_origin());
            self.drag = press.and_then(|press| {
                let (centre, corner, turn) = Self::handles(&placed, &placed.mapping, camera, rect)?;
                let anchor = placed.anchor(&placed.mapping)?;
                let grab = if press.distance(corner) <= REACH {
                    Grab::Size { from: press.distance(centre).max(1.0) }
                } else if press.distance(turn) <= REACH {
                    Grab::Turn { from: (press - centre).angle() }
                } else {
                    Grab::Move { at: Self::texture_at(&placed, &placed.mapping, project, orchestrator, camera, rect, press)? }
                };
                Some(Drag { grab, start: placed.mapping, centre, anchor })
            });
        }
        if let (Some(drag), true) = (self.drag, response.dragged_by(egui::PointerButton::Primary)) {
            if let Some(at) = response.interact_pointer_pos() {
                let next = match drag.grab {
                    Grab::Move { at: grabbed } => Self::texture_at(&placed, &drag.start, project, orchestrator, camera, rect, at).map(|now| moved(&drag.start, grabbed, now)),
                    Grab::Size { from } => Some(sized(&drag.start, at.distance(drag.centre).max(1.0) / from, drag.anchor)),
                    Grab::Turn { from } => {
                        let swept = (at - drag.centre).angle() - from;
                        // The way round that brings the circle toward the
                        // cursor: which that is depends on the projection,
                        // the side it is seen from and any flips.
                        let target_point = at;
                        [turned(&drag.start, swept, drag.anchor), turned(&drag.start, -swept, drag.anchor)]
                            .into_iter()
                            .min_by(|a, b| {
                                let reach = |m: &Mapping| Self::handles(&placed, m, camera, rect).map_or(f32::INFINITY, |(_, _, t)| t.distance(target_point));
                                reach(a).total_cmp(&reach(b))
                            })
                    }
                };
                if let Some(next) = next.filter(|n| *n != placed.mapping) {
                    apply(project, target, next);
                }
            }
        }
        true
    }

    /// The projection's shape, one repeat outlined on it, the handles, and
    /// a line of instruction. Painted after the viewport's image.
    pub fn draw(&self, ui: &egui::Ui, rect: egui::Rect, camera: &Camera, project: &Project) {
        let Some((_, placed)) = self.placed(project, camera) else { return };
        let painter = ui.painter_at(rect);
        let amber = egui::Color32::from_rgb(255, 196, 88);
        let faint = egui::Color32::from_rgba_unmultiplied(255, 196, 88, 120);
        let screen = |p: Vec3| gizmo_ui::project_point(camera, rect, placed.world.point(p));
        let polyline = |points: &[Vec3], stroke: egui::Stroke| {
            for pair in points.windows(2) {
                // Both ends in front of the camera, or a line across the view.
                if let (Some(a), Some(b)) = (screen(pair[0]), screen(pair[1])) {
                    painter.line_segment([a, b], stroke);
                }
            }
        };
        let lines = decal_fit::projection_lines(&placed.mapping, placed.bounds, Some(placed.viewer));
        for (i, line) in lines.iter().enumerate() {
            // A wrap's seam first, drawn stronger: where the texture's edges
            // meet.
            let seam = i == 0 && matches!(placed.mapping.projection, Projection::Cylindrical | Projection::Spherical);
            polyline(line, egui::Stroke::new(if seam { 1.8_f32 } else { 1.0_f32 }, if seam { amber } else { faint }));
        }
        // The repeat that holds the anchor, where the texture repeats; a
        // clamped one is a single piece, drawn where it is.
        let repeats = |w: tequila_texture::mip::Wrap| w == tequila_texture::mip::Wrap::Repeat;
        let anchor = placed.anchor(&placed.mapping).unwrap_or((0.5, 0.5));
        let whole = |t: f32, wraps: bool| if wraps { t.floor() } else { 0.0 };
        let shown = moved(&placed.mapping, (0.0, 0.0), (whole(anchor.0, repeats(placed.mapping.wrap_u)), whole(anchor.1, repeats(placed.mapping.wrap_v))));
        if let Some(mut outline) = decal_fit::tile_outline(&shown, placed.bounds, Some(placed.viewer), 16) {
            if let Some(&first) = outline.first() {
                outline.push(first);
            }
            polyline(&outline, egui::Stroke::new(2.0_f32, amber));
        }
        if let Some((centre, corner, turn)) = Self::handles(&placed, &placed.mapping, camera, rect) {
            let dark = egui::Stroke::new(1.0_f32, egui::Color32::from_black_alpha(140));
            painter.line_segment([centre - egui::vec2(6.0, 0.0), centre + egui::vec2(6.0, 0.0)], egui::Stroke::new(2.0_f32, amber));
            painter.line_segment([centre - egui::vec2(0.0, 6.0), centre + egui::vec2(0.0, 6.0)], egui::Stroke::new(2.0_f32, amber));
            painter.rect(egui::Rect::from_center_size(corner, egui::vec2(10.0, 10.0)), 2.0, amber, dark);
            painter.circle(turn, 6.0, amber, dark);
        }
        crate::label_gizmo::banner(&painter, rect, "Placing the texture · drag on the part to move · square to size · circle to turn · Esc to finish");
    }
}

fn apply(project: &mut Project, target: Target, mapping: Mapping) {
    if let Err(error) = project.apply(Command::SetTextureMapping { material: target.material, param: target.param, mapping: Box::new(mapping) }) {
        project.status = error.to_string();
    }
}

/// The offset that puts texture coordinates `grabbed` under the point that
/// reads `now` through `start`: the texture follows the cursor.
pub fn moved(start: &Mapping, grabbed: (f32, f32), now: (f32, f32)) -> Mapping {
    let mut next = *start;
    let sign = |flip: bool| if flip { -1.0 } else { 1.0 };
    next.offset.x += (grabbed.0 - now.0) * sign(start.flip_u);
    next.offset.y += (grabbed.1 - now.1) * sign(start.flip_v);
    next
}

/// `start` with its repeat `factor` times as big, about the texture
/// coordinates `anchor`.
pub fn sized(start: &Mapping, factor: f32, anchor: (f32, f32)) -> Mapping {
    let factor = factor.clamp(1e-3, 1e3);
    let mut next = *start;
    // A scale is repeats across the part, or with real-world units the
    // size of one: a bigger repeat is fewer of them, or a larger size.
    let by = if start.real_world_units { factor } else { 1.0 / factor };
    next.scale.x = (start.scale.x * by).clamp(1e-4, 1e5);
    next.scale.y = (start.scale.y * by).clamp(1e-4, 1e5);
    kept_in_place(start, next, anchor)
}

/// `start` turned by `radians`, about the texture coordinates `anchor`.
pub fn turned(start: &Mapping, radians: f32, anchor: (f32, f32)) -> Mapping {
    let mut next = *start;
    next.rotation = start.rotation + radians / std::f32::consts::TAU;
    kept_in_place(start, next, anchor)
}

/// `next` moved so the point that read `anchor` through `start` still does.
fn kept_in_place(start: &Mapping, next: Mapping, anchor: (f32, f32)) -> Mapping {
    let (a, b) = start.unapply(anchor.0, anchor.1);
    let (u, v) = next.apply_to(a, b);
    moved(&next, anchor, (u, v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};
    use tequila_texture::projection::Axis;

    fn mapping() -> Mapping {
        Mapping { projection: Projection::Planar, axis: Axis::Z, scale: vec3(3.0, 2.0, 1.0), rotation: 0.05, offset: vec2(0.2, -0.1), ..Default::default() }
    }

    /// Moving puts the grabbed texture point under the cursor; sizing and
    /// turning keep the anchor where it was, flipped or not.
    #[test]
    fn the_handles_move_size_and_turn_in_place() {
        for flips in [(false, false), (true, false), (false, true), (true, true)] {
            let start = Mapping { flip_u: flips.0, flip_v: flips.1, ..mapping() };
            // A point on the part, in the projection's own coordinates.
            let (a, b) = (0.31, 0.47);
            let grabbed = start.apply_to(a, b);
            let elsewhere = start.apply_to(a + 0.1, b - 0.05);
            let next = moved(&start, grabbed, elsewhere);
            let reads = next.apply_to(a + 0.1, b - 0.05);
            assert!((reads.0 - grabbed.0).abs() < 1e-4 && (reads.1 - grabbed.1).abs() < 1e-4, "{flips:?}: the texture did not follow: {reads:?} vs {grabbed:?}");

            let anchor = (1.7, -0.3);
            let held = start.unapply(anchor.0, anchor.1);
            for next in [sized(&start, 2.0, anchor), turned(&start, 0.4, anchor)] {
                let reads = next.apply_to(held.0, held.1);
                assert!((reads.0 - anchor.0).abs() < 1e-4 && (reads.1 - anchor.1).abs() < 1e-4, "{flips:?}: the anchor moved: {reads:?}");
            }
            assert!((sized(&start, 2.0, anchor).scale.x - start.scale.x / 2.0).abs() < 1e-5, "twice as big is half the repeats");
            let real = Mapping { real_world_units: true, ..start };
            assert!((sized(&real, 2.0, anchor).scale.x - real.scale.x * 2.0).abs() < 1e-5, "or twice the size");
        }
    }

    /// On the sample sphere, framed as the app frames it, the handles of
    /// every projection are in the view, whatever the repeat's size.
    #[test]
    fn the_handles_are_in_view_on_every_projection() {
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.25);
        let project = Project::with_document(document, std::sync::Arc::new(tequila_scene::demo::studio_environment()));
        let part = project.document().tree.iter().find(|n| n.mesh().is_some()).unwrap();
        let (node, bounds) = (part.id, project.document().geometry.get(part.mesh().unwrap()).unwrap().bounds());
        let world = project.document().tree.world_transform(node);
        let centre = world.point((bounds.min + bounds.max) * 0.5);
        // Far enough out to see the part whole, as framing it does.
        let size = (world.point(bounds.max) - world.point(bounds.min)).length();
        let camera = Camera::look_at(centre + vec3(0.0, 0.3, 1.0).normalize() * size * 1.6, centre, Vec3::Y);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 700.0));
        for projection in [Projection::Planar, Projection::Cylindrical, Projection::Spherical, Projection::TriPlanar] {
            for scale in [1.0, 3.0, 12.0] {
                let mapping = Mapping { projection, axis: Axis::Y, scale: vec3(scale * 2.0, scale, 1.0), ..Default::default() };
                let placed = Placed { mapping, node, bounds, world, viewer: world.inv_point(camera.position()) };
                let (middle, square, circle) = MappingGizmo::handles(&placed, &mapping, &camera, rect).expect("handles");
                for p in [middle, square, circle] {
                    assert!(rect.contains(p), "{projection:?} at {scale}: a handle at {p:?} is out of the view (middle {middle:?})");
                }
            }
        }
    }
}
