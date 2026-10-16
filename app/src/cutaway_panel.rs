//! Cutaways in the interface (`docs/02` §3, "Cutaway").
//!
//! A cutaway is added from the Scene tab like a light, sized to the model
//! and turned to the camera so it cuts something the moment it exists, then
//! moved and scaled with the transform gizmo like any node. Its own settings
//! — the shape, whether it is on, and what the cut face looks like — follow
//! the selection in the Material tab, as a light's do.

use crate::project::Project;
use eframe::egui;
use tequila_core::spectrum::Spectrum;
use tequila_scene::cutaway::{placement, CapStyle, CutShape, CutawaySettings};
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::Command;

/// The Add cutaway menu's entries. `None` until one is chosen.
pub fn add_cutaway_menu(ui: &mut egui::Ui, project: &Project, parent: Option<NodeId>) -> Option<Command> {
    let mut chosen = None;
    for (shape, hint) in [
        (CutShape::Plane, "A section: removes the half of the model nearest the camera."),
        (CutShape::Box, "Takes the corner nearest the camera out of the model."),
        (CutShape::Sphere, "A round scoop out of the front."),
        (CutShape::Cylinder, "A bore straight through, along the view."),
    ] {
        if ui.button(shape.label()).on_hover_text(hint).clicked() {
            let bounds = project.document().bounds();
            let eye = project.camera().camera.position();
            let center = if bounds.is_empty() { tequila_core::math::Vec3::ZERO } else { bounds.center() };
            let toward = eye - center;
            chosen = Some(Command::AddCutaway {
                parent,
                name: format!("{} cutaway", shape.label()),
                settings: Box::new(CutawaySettings {
                    shape,
                    ..Default::default()
                }),
                transform: placement(shape, bounds, toward),
            });
            ui.close_menu();
        }
    }
    ui.separator();
    ui.weak("Or give a part the Cutaway material: its own shape cuts.");
    chosen
}

/// The settings of the selected cutaway, if the selection is one.
pub fn cutaway_panel(ui: &mut egui::Ui, project: &mut Project) -> bool {
    let Some(node) = project.selection.active() else {
        return false;
    };
    let Some(NodeKind::Cutaway { settings }) = project.document().tree.get(node).map(|n| &n.kind) else {
        return false;
    };
    let before = (**settings).clone();
    let mut cutaway = before.clone();

    ui.add(crate::field::Checkbox::new(&mut cutaway.enabled, "Cutting"))
        .on_hover_text("Off, the model is whole again; the cutaway stays where it is.");

    ui.horizontal(|ui| {
        ui.label("Shape");
        for shape in CutShape::ALL {
            ui.selectable_value(&mut cutaway.shape, shape, shape.label());
        }
    });

    ui.horizontal(|ui| {
        ui.label("Cut face");
        let own = matches!(cutaway.cap, CapStyle::Own);
        if ui
            .selectable_label(own, "Part's material")
            .on_hover_text("The cut face looks like the part it cuts, as if solid all the way through.")
            .clicked()
        {
            cutaway.cap = CapStyle::Own;
        }
        if ui
            .selectable_label(!own, "Colour")
            .on_hover_text("One flat colour for every cut face, the way a drawing marks a section.")
            .clicked()
            && own
        {
            cutaway.cap = CapStyle::Color(Spectrum {
                r: 0.85,
                g: 0.2,
                b: 0.15,
            });
        }
        if let CapStyle::Color(color) = &mut cutaway.cap {
            crate::color_picker::edit(ui, color);
        }
    });

    ui.small(
        "Move, turn and scale it with the transform gizmo. Everything inside it is \
         removed; a plane removes what is in front of it.",
    );

    if cutaway != before {
        let _ = project.apply(Command::SetCutaway {
            node,
            settings: Box::new(cutaway),
        });
    }
    true
}

/// The outline of every cutaway in the viewport: the volume a node removes,
/// so it can be seen and placed. A cutter is invisible in the render by
/// nature — that is its job — and one that could not be seen in the viewport
/// either could only be found through the Scene tab.
pub fn draw_outlines(ui: &egui::Ui, rect: egui::Rect, camera: &tequila_render_core::camera::Camera, project: &Project) {
    use tequila_core::math::{vec3, Vec3};
    let document = project.document();
    let painter = ui.painter_at(rect);
    for node in document.tree.iter() {
        let NodeKind::Cutaway { settings } = &node.kind else {
            continue;
        };
        if !document.tree.is_effectively_visible(node.id) {
            continue;
        }
        let selected = project.selection.contains(node.id);
        let alpha = if settings.enabled { 1.0 } else { 0.4 };
        let colour = if selected {
            crate::theme::p().accent
        } else {
            egui::Color32::from_rgb(90, 200, 230)
        }
        .gamma_multiply(alpha);
        let stroke = egui::Stroke::new(if selected { 1.8_f32 } else { 1.1 }, colour);
        let world = document.tree.world_transform(node.id);

        // Lines in the cutter's own space.
        let mut lines: Vec<(Vec3, Vec3)> = Vec::new();
        let circle = |lines: &mut Vec<(Vec3, Vec3)>, at: &dyn Fn(f32) -> Vec3| {
            const STEPS: usize = 48;
            for i in 0..STEPS {
                let a = i as f32 / STEPS as f32 * std::f32::consts::TAU;
                let b = (i + 1) as f32 / STEPS as f32 * std::f32::consts::TAU;
                lines.push((at(a), at(b)));
            }
        };
        match settings.shape {
            CutShape::Plane => {
                let corners = [vec3(-1.0, -1.0, 0.0), vec3(1.0, -1.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(-1.0, 1.0, 0.0)];
                for i in 0..4 {
                    lines.push((corners[i], corners[(i + 1) % 4]));
                }
                // Which side goes: an arrow into the removed half.
                lines.push((Vec3::ZERO, vec3(0.0, 0.0, 0.5)));
                lines.push((vec3(0.0, 0.0, 0.5), vec3(0.08, 0.0, 0.4)));
                lines.push((vec3(0.0, 0.0, 0.5), vec3(-0.08, 0.0, 0.4)));
            }
            CutShape::Box => {
                for a in [-1.0f32, 1.0] {
                    for b in [-1.0f32, 1.0] {
                        lines.push((vec3(-1.0, a, b), vec3(1.0, a, b)));
                        lines.push((vec3(a, -1.0, b), vec3(a, 1.0, b)));
                        lines.push((vec3(a, b, -1.0), vec3(a, b, 1.0)));
                    }
                }
            }
            CutShape::Sphere => {
                circle(&mut lines, &|t| vec3(t.cos(), t.sin(), 0.0));
                circle(&mut lines, &|t| vec3(t.cos(), 0.0, t.sin()));
                circle(&mut lines, &|t| vec3(0.0, t.cos(), t.sin()));
            }
            CutShape::Cylinder => {
                circle(&mut lines, &|t| vec3(t.cos(), t.sin(), -1.0));
                circle(&mut lines, &|t| vec3(t.cos(), t.sin(), 1.0));
                for t in [0.0f32, 0.25, 0.5, 0.75] {
                    let a = t * std::f32::consts::TAU;
                    lines.push((vec3(a.cos(), a.sin(), -1.0), vec3(a.cos(), a.sin(), 1.0)));
                }
            }
        }
        for (a, b) in lines {
            let (Some(a), Some(b)) = (
                crate::gizmo_ui::project_point(camera, rect, world.point(a)),
                crate::gizmo_ui::project_point(camera, rect, world.point(b)),
            ) else {
                continue;
            };
            painter.line_segment([a, b], stroke);
        }
    }
}
