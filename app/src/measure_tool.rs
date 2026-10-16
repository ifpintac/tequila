//! The measurements tool (`TEQUILA_SPEC.md` §1.11).
//!
//! Click two points on the model and the distance between them is drawn
//! over the viewport in the scene's own unit, and kept with the project. The
//! pointer snaps: to a corner when one is near, then to the middle of an
//! edge, then onto an edge, and otherwise to the surface under it — the
//! order a draughtsman would pick, so a dimension lands on the feature it is
//! meant to measure without a steady hand. The snapping itself is
//! [`tequila_scene::measure`]'s; this is the pointer handling and drawing
//! around it.

use crate::gizmo_ui;
use crate::orchestrator::Orchestrator;
use crate::project::Project;
use eframe::egui;
use tequila_core::math::Vec3;
use tequila_render_core::camera::Camera;
use tequila_render_core::scene::Mesh;
use tequila_scene::measure::{format_length, snap_candidates, Anchor, FeatureEdges, Measurement, SnapKind};
use tequila_scene::Command;
use std::collections::HashMap;
use std::sync::Arc;

/// How near the pointer a snap point must land, in points.
const SNAP_RADIUS: f32 = 14.0;

#[derive(Default)]
pub struct MeasureTool {
    active: bool,
    /// The first end, once placed.
    first: Option<Anchor>,
    /// Where a click would land now, and what it would be on.
    hover: Option<(Anchor, Vec3, SnapKind)>,
    /// Each mesh's edges, found once. Keyed by the mesh's allocation, and
    /// holding the mesh so the key cannot be reused by another.
    features: HashMap<usize, (Arc<Mesh>, Arc<FeatureEdges>)>,
}

impl MeasureTool {
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn toggle(&mut self) {
        self.active = !self.active;
        self.first = None;
        self.hover = None;
    }

    fn features_of(&mut self, mesh: &Arc<Mesh>) -> Arc<FeatureEdges> {
        let key = Arc::as_ptr(mesh) as usize;
        if self.features.len() > 256 {
            self.features.clear();
        }
        self.features
            .entry(key)
            .or_insert_with(|| (mesh.clone(), Arc::new(FeatureEdges::of(mesh))))
            .1
            .clone()
    }

    /// Where a click at `at` would put an end.
    fn snap(
        &mut self,
        rect: egui::Rect,
        camera: &Camera,
        project: &Project,
        orchestrator: &Orchestrator,
        at: egui::Pos2,
    ) -> Option<(Anchor, Vec3, SnapKind)> {
        let ray = gizmo_ui::ray_through(camera, rect, at)?;
        let hit = orchestrator.pick(&ray)?;
        let node = project.mapping().node_of(hit.instance)?;
        let scene = project.scene();
        let instance = scene.instances.get(hit.instance.0 as usize)?;
        let transform = instance.transform;
        let local = transform.inv_point(hit.position);

        let mut best = (local, SnapKind::Surface, f32::INFINITY);
        if let Some(mesh) = scene.meshes.get(instance.mesh.index()).cloned() {
            let features = self.features_of(&mesh);
            for (point, kind) in snap_candidates(&mesh, &features, hit.primitive as usize, local) {
                let Some(screen) = gizmo_ui::project_point(camera, rect, transform.point(point)) else {
                    continue;
                };
                let distance = screen.distance(at);
                if distance > SNAP_RADIUS {
                    continue;
                }
                // The better kind wins; between two of a kind, the nearer.
                if (kind, distance) < (best.1, best.2) {
                    best = (point, kind, distance);
                }
            }
        }
        let anchor = Anchor {
            node: Some(node),
            point: best.0,
        };
        Some((anchor, transform.point(best.0), best.1))
    }

    /// Take a click, if measuring. Drags are left alone, so the view can
    /// still be turned between the two ends.
    pub fn interact(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        camera: &Camera,
        project: &mut Project,
        orchestrator: &Orchestrator,
        response: &egui::Response,
    ) -> bool {
        if !self.active {
            return false;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            // The first Escape drops a half-made measurement; the second
            // leaves the tool.
            if self.first.take().is_none() {
                self.active = false;
                project.status = "Measuring finished".into();
            }
            self.hover = None;
            return true;
        }
        self.hover = response
            .hover_pos()
            .and_then(|at| self.snap(rect, camera, project, orchestrator, at));
        if !response.clicked() {
            return false;
        }
        let Some((anchor, _, _)) = self.hover else {
            return true;
        };
        match self.first.take() {
            None => self.first = Some(anchor),
            Some(first) => {
                let mut measurements = project.document().measurements.clone();
                let measurement = Measurement::new(first, anchor);
                let length = measurement.length(project.document());
                measurements.push(measurement);
                match project.apply(Command::SetMeasurements { measurements }) {
                    Ok(()) => {
                        if let Some(length) = length {
                            project.status = format!("Measured {}", format_length(length, &project.document().units));
                        }
                    }
                    Err(error) => project.status = error.to_string(),
                }
            }
        }
        true
    }

    /// Every visible measurement, and while measuring the snap point and the
    /// dimension being drawn.
    pub fn draw(&self, ui: &egui::Ui, rect: egui::Rect, camera: &Camera, project: &Project) {
        let painter = ui.painter_at(rect);
        let document = project.document();
        let line = egui::Color32::from_rgb(255, 214, 90);
        for measurement in document.measurements.iter().filter(|m| m.visible) {
            let (Some((a, b)), Some(length)) = (measurement.ends(document), measurement.length(document)) else {
                continue;
            };
            dimension(&painter, rect, camera, a, b, &format_length(length, &document.units), line);
        }
        if !self.active {
            return;
        }

        let hint = if self.first.is_some() {
            "Click the second point · Esc to start again"
        } else {
            "Click the first point to measure from · Esc to finish"
        };
        painter.text(
            rect.center_top() + egui::vec2(0.0, 14.0),
            egui::Align2::CENTER_TOP,
            hint,
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );

        if let (Some(first), Some((_, to, _))) = (self.first, self.hover) {
            if let Some(from) = first.world(&document.tree) {
                let length = format_length((to - from).length(), &document.units);
                dimension(&painter, rect, camera, from, to, &length, line.gamma_multiply(0.7));
            }
        }
        if let Some((_, at, kind)) = self.hover {
            if let Some(screen) = gizmo_ui::project_point(camera, rect, at) {
                snap_marker(&painter, screen, kind);
                painter.text(
                    screen + egui::vec2(10.0, -10.0),
                    egui::Align2::LEFT_BOTTOM,
                    kind.label(),
                    egui::FontId::proportional(11.0),
                    egui::Color32::from_rgb(120, 220, 255),
                );
            }
        }
    }
}

/// A dimension line between two world points, with its length in the middle.
fn dimension(
    painter: &egui::Painter,
    rect: egui::Rect,
    camera: &Camera,
    a: Vec3,
    b: Vec3,
    label: &str,
    colour: egui::Color32,
) {
    let (Some(pa), Some(pb)) = (
        gizmo_ui::project_point(camera, rect, a),
        gizmo_ui::project_point(camera, rect, b),
    ) else {
        return;
    };
    // Dark under light, so the line reads over a white part and a black one.
    painter.line_segment([pa, pb], egui::Stroke::new(3.5_f32, egui::Color32::from_black_alpha(160)));
    painter.line_segment([pa, pb], egui::Stroke::new(1.6_f32, colour));
    for p in [pa, pb] {
        painter.circle_filled(p, 3.5, colour);
    }
    let middle = pa + (pb - pa) * 0.5;
    let galley = painter.layout_no_wrap(label.to_string(), egui::FontId::proportional(12.5), egui::Color32::BLACK);
    let size = galley.size() + egui::vec2(10.0, 4.0);
    let frame = egui::Rect::from_center_size(middle, size);
    painter.rect_filled(frame, 3.0, colour);
    painter.galley(frame.min + egui::vec2(5.0, 2.0), galley, egui::Color32::BLACK);
}

/// A different mark for each kind of snap, so it is plain before the click
/// what the click will land on.
fn snap_marker(painter: &egui::Painter, at: egui::Pos2, kind: SnapKind) {
    let colour = egui::Color32::from_rgb(120, 220, 255);
    let stroke = egui::Stroke::new(2.0_f32, colour);
    match kind {
        SnapKind::Vertex => {
            painter.rect_stroke(egui::Rect::from_center_size(at, egui::vec2(11.0, 11.0)), 0.0, stroke);
        }
        SnapKind::Midpoint => {
            let r = 7.0;
            let points = vec![at + egui::vec2(0.0, -r), at + egui::vec2(r, r * 0.7), at + egui::vec2(-r, r * 0.7)];
            painter.add(egui::Shape::closed_line(points, stroke));
        }
        SnapKind::Edge => {
            painter.circle_stroke(at, 6.0, stroke);
        }
        SnapKind::Surface => {
            painter.circle_filled(at, 3.0, colour);
        }
    }
}

/// The Measurements section of the Scene tab: the tool's switch and the list.
pub fn panel(ui: &mut egui::Ui, project: &mut Project, tool: &mut MeasureTool) {
    ui.horizontal(|ui| {
        if ui
            .add(egui::SelectableLabel::new(tool.active(), "📏 Measure"))
            .on_hover_text(
                "Click two points on the model. The pointer snaps to corners and \
                 edges; Esc finishes.",
            )
            .clicked()
        {
            tool.toggle();
        }
        let any = !project.document().measurements.is_empty();
        if ui.add_enabled(any, egui::Button::new("Clear")).clicked() {
            let _ = project.apply(Command::SetMeasurements {
                measurements: Vec::new(),
            });
        }
    });

    let original = project.document().measurements.clone();
    let mut measurements = original.clone();
    let mut remove = None;
    for (index, measurement) in measurements.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let seen = measurement.visible;
            if ui
                .add(egui::SelectableLabel::new(seen, if seen { "👁" } else { "🚫" }))
                .on_hover_text("Show or hide it in the viewport")
                .clicked()
            {
                measurement.visible = !seen;
            }
            let text = match measurement.length(project.document()) {
                Some(length) => format_length(length, &project.document().units),
                None => "its part was deleted".into(),
            };
            ui.label(text);
            if ui.small_button("×").on_hover_text("Remove").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        measurements.remove(index);
    }
    if measurements != original {
        let _ = project.apply(Command::SetMeasurements { measurements });
    }
}
