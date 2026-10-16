//! Drawing the transform gizmo and driving a drag.
//!
//! The arithmetic lives in [`tequila_scene::gizmo`], which is where it can be
//! tested. This is the part that cannot be: turning cursor positions into
//! rays, painting handles, and deciding when a drag becomes a command.
//!
//! One decision worth stating. A drag issues `SetTransform` on **every frame**
//! it moves, and those coalesce into a single undo step because
//! `Command::can_merge_with` merges consecutive `SetTransform`s on the same
//! node. That means the viewport shows the real, path-traced result while the
//! object is still moving — §1.1's "the viewport is the renderer" — rather
//! than a ghost that snaps into place on release.

use crate::project::Project;
use eframe::egui;
use tequila_core::math::{vec2, Ray, Transform, Vec3};
use tequila_render_core::camera::Camera;
use tequila_scene::gizmo::{self, Axis, Drag, GizmoFrame, GizmoMode, Handle, PivotMode, Snapping};
use tequila_scene::tree::NodeId;
use tequila_scene::Command;

#[derive(Debug)]
pub struct GizmoUi {
    pub mode: GizmoMode,
    pub pivot_mode: PivotMode,
    pub snapping: Snapping,
    /// World space when false. §5's world/local toggle.
    pub local_space: bool,
    drag: Option<ActiveDrag>,
    hovered: Option<Handle>,
    /// What [`GizmoUi::draw`] will paint, worked out by [`GizmoUi::interact`].
    ///
    /// The two are separate calls because the gizmo has to *read* the pointer
    /// before the viewport decides whether a drag is an orbit, and has to
    /// *paint* after the rendered image — egui draws in call order within a
    /// layer, so a gizmo painted first is a gizmo painted underneath the
    /// picture and invisible.
    pending: Option<(GizmoFrame, Vec3)>,
    /// The targets and pivot for a selection, at a count of the snapshot's
    /// rebuilds: worked out once, not every frame - a selected group of a
    /// big assembly walked every part under it sixty times a second.
    worked_out: Option<WorkedOut>,
}

#[derive(Debug)]
struct WorkedOut {
    selected: Vec<NodeId>,
    rebuilds: u64,
    pivot_mode: PivotMode,
    targets: Vec<NodeId>,
    pivot: Option<Vec3>,
}

/// A drag in progress, over however many nodes are selected.
///
/// The gizmo's own state is shared; only the starting transform differs per
/// node, so each one is re-derived from the same cursor motion.
#[derive(Clone, Debug)]
struct ActiveDrag {
    gizmo: Drag,
    starts: Vec<(NodeId, Transform)>,
}

impl Default for GizmoUi {
    fn default() -> Self {
        GizmoUi {
            mode: GizmoMode::Move,
            pivot_mode: PivotMode::BoundingBoxCenter,
            snapping: Snapping {
                grid: None,
                angle: Some(15f32.to_radians()),
                scale: None,
                vertex_within: None,
            },
            local_space: false,
            drag: None,
            hovered: None,
            pending: None,
            worked_out: None,
        }
    }
}

impl GizmoUi {
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// The nodes the gizmo acts on: the selection, minus anything whose
    /// ancestor is also selected. See
    /// [`tequila_scene::selection::transform_targets`] for why.
    fn targets(&mut self, project: &Project) -> (Vec<NodeId>, Option<Vec3>) {
        let selected: Vec<NodeId> = project.selection.nodes().collect();
        let fresh = self.worked_out.as_ref().is_some_and(|w| {
            w.selected == selected && w.rebuilds == project.rebuilds() && w.pivot_mode == self.pivot_mode
        });
        if !fresh {
            let targets = tequila_scene::selection::transform_targets(project.document(), &selected);
            let pivot = gizmo::pivot_for(project.document(), &targets, self.pivot_mode, None);
            self.worked_out = Some(WorkedOut { selected, rebuilds: project.rebuilds(), pivot_mode: self.pivot_mode, targets, pivot });
        }
        let worked = self.worked_out.as_ref().expect("worked out above");
        (worked.targets.clone(), worked.pivot)
    }

    /// Read the pointer and drive a drag. Returns true if the pointer is over
    /// a handle, so the viewport knows not to start an orbit.
    ///
    /// Paints nothing. Call [`GizmoUi::draw`] after the rendered image.
    pub fn interact(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        camera: &Camera,
        project: &mut Project,
        response: &egui::Response,
    ) -> bool {
        self.pending = None;
        let (targets, pivot) = self.targets(project);
        let Some(&first) = targets.first() else {
            self.drag = None;
            self.hovered = None;
            return false;
        };
        let Some(pivot) = pivot else {
            return false;
        };

        let view = camera.forward();
        let distance = (pivot - camera.position()).length().max(1e-3);
        // Constant size on screen: a gizmo that shrinks with distance becomes
        // unclickable on a large assembly.
        let handle_length = distance * 0.14;

        // Local space follows the first selected node's orientation. With
        // several selected there is no single "local", and picking one is less
        // surprising than silently falling back to world.
        let frame = if self.local_space {
            GizmoFrame::local(
                pivot,
                project.document().tree.world_transform(first),
                handle_length,
            )
        } else {
            GizmoFrame::world(pivot, handle_length)
        };

        let pointer = ui.input(|i| i.pointer.hover_pos());
        let ray = pointer.and_then(|p| ray_through(camera, rect, p));

        // --- drag ---------------------------------------------------------
        if let Some(drag) = &self.drag {
            if response.dragged() {
                if let Some(ray) = &ray {
                    // Vertex snapping is resolved once, against the node that
                    // was grabbed, and the resulting correction is applied to
                    // every node. Snapping each one to its own nearest vertex
                    // would pull the parts apart: the selection would stop
                    // moving rigidly and start deforming.
                    let correction = self.vertex_snap_correction(project, drag, ray);

                    // One command for the whole selection, so the move is one
                    // undo step whether it touched one part or forty.
                    let transforms: Vec<(NodeId, Transform)> = drag
                        .starts
                        .iter()
                        .map(|(node, start)| {
                            let step = gizmo::Drag {
                                start: *start,
                                ..drag.gizmo
                            };
                            let moved = gizmo::update(&step, ray, &self.snapping);
                            (*node, world_to_local(project, *node, moved, correction))
                        })
                        .collect();
                    let _ = project.apply(Command::SetTransform { transforms });
                }
            } else {
                // The drag is over: the next transform, from wherever, is a
                // separate undo step rather than more of this one.
                self.drag = None;
                project.end_interaction();
            }
        } else if response.drag_started() {
            let press_ray = ui.input(|i| i.pointer.press_origin()).and_then(|p| ray_through(camera, rect, p));
            if let Some((ray, handle)) = press_ray.as_ref().and_then(|ray| gizmo::pick(ray, &frame, self.mode, view).map(|handle| (ray, handle))) {
                let starts: Vec<(NodeId, Transform)> = targets
                    .iter()
                    .map(|id| (*id, project.document().tree.world_transform(*id)))
                    .collect();
                // Every node's delta is measured from the same grab, so the
                // selection moves rigidly rather than each part converging on
                // the cursor.
                let gizmo = gizmo::begin(
                    handle,
                    self.mode,
                    frame,
                    starts
                        .first()
                        .map(|(_, t)| *t)
                        .unwrap_or(Transform::IDENTITY),
                    ray,
                    view,
                );
                self.drag = Some(ActiveDrag { gizmo, starts });
            }
        }

        self.hovered = match (&ray, &self.drag) {
            (_, Some(drag)) => Some(drag.gizmo.handle),
            (Some(ray), None) => gizmo::pick(ray, &frame, self.mode, view),
            _ => None,
        };

        self.pending = Some((frame, view));
        self.hovered.is_some()
    }

    /// Paint the handles worked out by the last [`GizmoUi::interact`].
    ///
    /// Call this *after* the rendered image, or the image covers it.
    pub fn draw(&self, ui: &mut egui::Ui, rect: egui::Rect, camera: &Camera) {
        if let Some((frame, view)) = self.pending {
            self.paint(ui, rect, camera, &frame, view);
        }
    }

    /// §5's vertex snapping, as one world-space correction for the whole drag.
    ///
    /// Resolved against the node the drag started on, and returned rather than
    /// applied, so every node in the selection receives the same shift and the
    /// selection moves as one rigid body.
    fn vertex_snap_correction(&self, project: &Project, drag: &ActiveDrag, ray: &Ray) -> Vec3 {
        let Some(radius) = self.snapping.vertex_within.filter(|r| *r > 0.0) else {
            return Vec3::ZERO;
        };
        let Some((_, start)) = drag.starts.first() else {
            return Vec3::ZERO;
        };
        let step = gizmo::Drag {
            start: *start,
            ..drag.gizmo
        };
        let moved = gizmo::update(&step, ray, &self.snapping);

        // Where that node's origin would land, in world space.
        let landed = moved.point(Vec3::ZERO);
        match gizmo::nearest_vertex(project.document(), landed, radius) {
            Some(snapped) => snapped - landed,
            None => Vec3::ZERO,
        }
    }

    fn paint(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        camera: &Camera,
        frame: &GizmoFrame,
        view: Vec3,
    ) {
        let painter = ui.painter_at(rect);
        let Some(centre) = project_point(camera, rect, frame.pivot) else {
            return;
        };

        let colour = |axis: Axis, handle: Handle| {
            let base = match axis {
                Axis::X => egui::Color32::from_rgb(230, 80, 80),
                Axis::Y => egui::Color32::from_rgb(110, 210, 110),
                Axis::Z => egui::Color32::from_rgb(90, 150, 250),
            };
            if self.hovered == Some(handle) {
                egui::Color32::from_rgb(255, 220, 80)
            } else {
                base
            }
        };

        match self.mode {
            GizmoMode::Rotate => {
                for axis in Axis::ALL {
                    let normal = frame.direction(axis);
                    let (u, v) = perpendicular(normal);
                    let mut points = Vec::with_capacity(65);
                    for i in 0..=64 {
                        let a = i as f32 / 64.0 * std::f32::consts::TAU;
                        let world = frame.pivot + (u * a.cos() + v * a.sin()) * frame.handle_length;
                        // The far half of a ring is hidden by the object it
                        // surrounds; drawing it makes the near and far edges
                        // indistinguishable under the cursor.
                        if (world - frame.pivot).dot(view) > 0.0 {
                            if points.len() > 1 {
                                painter.add(egui::Shape::line(
                                    std::mem::take(&mut points),
                                    egui::Stroke::new(2.0_f32, colour(axis, Handle::Axis(axis))),
                                ));
                            }
                            points.clear();
                            continue;
                        }
                        if let Some(point) = project_point(camera, rect, world) {
                            points.push(point);
                        }
                    }
                    if points.len() > 1 {
                        painter.add(egui::Shape::line(
                            points,
                            egui::Stroke::new(2.0_f32, colour(axis, Handle::Axis(axis))),
                        ));
                    }
                }
            }
            _ => {
                for axis in Axis::ALL {
                    let tip = frame.pivot + frame.direction(axis) * frame.handle_length;
                    if let Some(end) = project_point(camera, rect, tip) {
                        painter.text(end + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM,
                            axis.label(), egui::FontId::proportional(12.0), colour(axis, Handle::Axis(axis)));
                        let stroke = egui::Stroke::new(2.5_f32, colour(axis, Handle::Axis(axis)));
                        painter.line_segment([centre, end], stroke);
                        // A square cap for scale, an arrowhead for move — the
                        // conventional distinction, and the only thing on
                        // screen that says which mode is active.
                        if self.mode == GizmoMode::Scale {
                            painter.rect_filled(
                                egui::Rect::from_center_size(end, egui::vec2(8.0, 8.0)),
                                1.0,
                                stroke.color,
                            );
                        } else {
                            painter.circle_filled(end, 4.5, stroke.color);
                        }
                    }
                    // The plane quad, in the inner corner between two axes.
                    let (u, v) = plane_axes(frame, axis);
                    let (low, high) = (frame.handle_length * 0.22, frame.handle_length * 0.55);
                    let corners: Option<Vec<egui::Pos2>> = [
                        frame.pivot + u * low + v * low,
                        frame.pivot + u * high + v * low,
                        frame.pivot + u * high + v * high,
                        frame.pivot + u * low + v * high,
                    ]
                    .iter()
                    .map(|p| project_point(camera, rect, *p))
                    .collect();
                    if let Some(corners) = corners {
                        let mut fill = colour(axis, Handle::Plane(axis));
                        fill = egui::Color32::from_rgba_unmultiplied(
                            fill.r(),
                            fill.g(),
                            fill.b(),
                            if self.hovered == Some(Handle::Plane(axis)) {
                                160
                            } else {
                                60
                            },
                        );
                        painter.add(egui::Shape::convex_polygon(
                            corners,
                            fill,
                            egui::Stroke::NONE,
                        ));
                    }
                }
                let screen = if self.hovered == Some(Handle::Screen) {
                    egui::Color32::from_rgb(255, 220, 80)
                } else {
                    egui::Color32::from_gray(200)
                };
                painter.circle_stroke(centre, 9.0, egui::Stroke::new(1.5_f32, screen));
            }
        }
    }
}

/// Apply a world-space shift to a node's local transform.
///
/// The transform is relative to the parent, so the shift has to be expressed
/// in the parent's frame — applying it directly would send a part inside a
/// rotated or scaled group off in the wrong direction.
fn world_to_local(project: &Project, node: NodeId, transform: Transform, shift: Vec3) -> Transform {
    let parent = project.document().tree.get(node).and_then(|n| n.parent);
    let parent_inverse = match parent {
        Some(parent) => project
            .document()
            .tree
            .world_transform(parent)
            .inverted(),
        None => Transform::IDENTITY,
    };
    Transform::from_matrix(parent_inverse.matrix * tequila_core::math::Mat4::translation(shift) * transform.matrix)
        .unwrap_or(transform)
}

#[cfg(test)]
mod transform_tests {
    use super::*;
    use tequila_core::math::{Mat4, vec3};

    /// The targets and pivot are worked out once per selection and scene,
    /// and again whenever either changes: a move, a change of selection, a
    /// different pivot - never a stale gizmo.
    #[test]
    fn the_worked_out_pivot_follows_the_scene_and_the_selection() {
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.25);
        let mut project = Project::with_document(document, std::sync::Arc::new(tequila_scene::demo::studio_environment()));
        let part = project.document().tree.iter().find(|n| n.material.is_some()).unwrap().id;
        let mut gizmo = GizmoUi::default();
        assert!(gizmo.targets(&project).0.is_empty(), "nothing selected, nothing to move");
        project.selection.select_many([part], tequila_scene::selection::SelectionMode::Replace);
        let (targets, before) = gizmo.targets(&project);
        assert_eq!(targets, vec![part]);
        let before = before.expect("a pivot");
        assert_eq!(gizmo.targets(&project).1, Some(before), "asked again, the same");
        let moved = project.document().tree.get(part).unwrap().transform * Transform::translation(vec3(0.0, 0.5, 0.0));
        project.apply(Command::SetTransform { transforms: vec![(part, moved)] }).unwrap();
        let after = gizmo.targets(&project).1.expect("a pivot");
        assert!((after.y - before.y - 0.5).abs() < 1e-4, "the pivot did not follow the move: {before:?} -> {after:?}");
        project.selection.clear();
        assert!(gizmo.targets(&project).0.is_empty(), "a cleared selection kept its targets");
    }

    #[test]
    fn world_edits_survive_rotated_scaled_parent_groups() {
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.25);
        let mut project = Project::with_document(document, std::sync::Arc::new(tequila_scene::demo::studio_environment()));
        let part = project.document().tree.iter().find(|n| n.material.is_some()).unwrap().id;
        project.apply(Command::AddGroup { parent: None, name: "Transform test group".into() }).unwrap();
        let parent = project.document().tree.iter().find(|n| n.name == "Transform test group").unwrap().id;
        project.apply(Command::Reparent { node: part, parent: Some(parent), index: None }).unwrap();
        let parent_matrix = Mat4::translation(vec3(3.0, 2.0, -1.0)) * Mat4::rotation(Vec3::Z, 0.7) * Mat4::scale(vec3(2.0, 0.5, 1.5));
        project.apply(Command::set_transform(parent, Transform::from_matrix(parent_matrix).unwrap())).unwrap();
        let start = project.document().tree.world_transform(part);
        for delta in [Mat4::translation(Vec3::X), Mat4::rotation(Vec3::Y, 0.4), Mat4::scale(vec3(1.0, 2.0, 1.0))] {
            let expected = Transform::from_matrix(delta * start.matrix).unwrap();
            let local = world_to_local(&project, part, expected, Vec3::ZERO);
            let actual = Transform::from_matrix(parent_matrix * local.matrix).unwrap();
            for p in [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z] {
                assert!((actual.point(p) - expected.point(p)).length() < 1e-4);
            }
        }
    }
}

/// Two unit vectors spanning the plane normal to `n`.
fn perpendicular(n: Vec3) -> (Vec3, Vec3) {
    let onb = tequila_core::math::Onb::from_normal(n);
    (onb.t, onb.b)
}

fn plane_axes(frame: &GizmoFrame, axis: Axis) -> (Vec3, Vec3) {
    match axis {
        Axis::X => (frame.direction(Axis::Y), frame.direction(Axis::Z)),
        Axis::Y => (frame.direction(Axis::Z), frame.direction(Axis::X)),
        Axis::Z => (frame.direction(Axis::X), frame.direction(Axis::Y)),
    }
}

/// A ray through a screen position, in the same parameterization the renderer
/// uses — so what the user clicks is what the renderer drew there.
pub fn ray_through(camera: &Camera, rect: egui::Rect, at: egui::Pos2) -> Option<Ray> {
    if !rect.contains(at) {
        return None;
    }
    let film = vec2(
        (at.x - rect.left()) / rect.width().max(1.0),
        (at.y - rect.top()) / rect.height().max(1.0),
    );
    let aspect = rect.width().max(1.0) / rect.height().max(1.0);
    // A centre-of-pixel, centre-of-aperture sample: picking must be
    // deterministic, and a jittered ray would select a different part on two
    // clicks in the same place.
    Some(camera.pick_ray(film, aspect))
}

/// Where a world point lands on screen, or `None` when it is behind the camera.
pub fn project_point(camera: &Camera, rect: egui::Rect, world: Vec3) -> Option<egui::Pos2> {
    let aspect = rect.width().max(1.0) / rect.height().max(1.0);
    let film = camera.project(world, aspect)?;
    Some(egui::pos2(
        rect.left() + film.x * rect.width(),
        rect.top() + film.y * rect.height(),
    ))
}
