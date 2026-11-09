//! Patterns in the interface: making one from a part (Edit › Pattern, the
//! viewport's and the Scene tab's right-click), and the Pattern section that
//! edits one (`tequila_scene::pattern`).

use crate::numeric;
use crate::project::Project;
use eframe::egui;
use tequila_core::math::{vec3, Vec3};
use tequila_scene::pattern::{self, PatternKind, MOST};
use tequila_scene::selection::SelectionMode;
use tequila_scene::tree::NodeId;
use tequila_scene::Command;

/// The part a new pattern would repeat: one part selected, not already in
/// a pattern.
pub fn candidate(project: &Project) -> Option<NodeId> {
    let selected: Vec<NodeId> = project.selection.nodes().collect();
    let [one] = selected[..] else { return None };
    let node = project.document().tree.get(one)?;
    (node.mesh().is_some() && pattern::pattern_of(project.document(), one).is_none()).then_some(one)
}

/// "Pattern along a line" and "Pattern around a circle", for a menu.
pub fn menu_items(ui: &mut egui::Ui, project: &mut Project) {
    let part = candidate(project);
    for (label, circle, about) in [
        ("Pattern along a line", false, "Copies of the part in a row, sharing its mesh and material"),
        ("Pattern around a circle", true, "Copies of the part around a circle, sharing its mesh and material"),
    ] {
        let button = ui.add_enabled(part.is_some(), egui::Button::new(label)).on_hover_text(about).on_disabled_hover_text("Select one part first");
        if button.clicked() {
            if let Some(part) = part {
                make(project, part, circle);
            }
            ui.close_menu();
        }
    }
}

/// Make a pattern of `part`: grouped, with copies laid out at once, as one
/// step to undo, and the group selected so its settings show.
pub fn make(project: &mut Project, part: NodeId, circle: bool) -> Option<NodeId> {
    let name = if circle { "Circle pattern" } else { "Line pattern" };
    project.begin_group();
    let made = (|| {
        project.apply(Command::Group { nodes: vec![part], name: name.into() }).ok()?;
        let group = project.document().tree.get(part)?.parent?;
        let start = pattern::starting(project.document(), part, circle)?.with_ids(project.document(), Some(group));
        project.apply(Command::SetPattern { group, pattern: Some(Box::new(start)) }).ok()?;
        Some(group)
    })();
    match made {
        Some(group) => {
            project.end_group();
            project.selection.select_many([group], SelectionMode::Replace);
            project.status = format!("{name}: change it under Pattern in the Scene tab");
        }
        None => {
            project.abandon_group();
            project.status = "Could not make a pattern of that part".into();
        }
    }
    made
}

/// The Pattern section, for the pattern `group` is.
pub fn panel(ui: &mut egui::Ui, project: &mut Project, group: NodeId) {
    let Some(current) = project.document().tree.get(group).and_then(|g| g.pattern.clone()) else { return };
    let shown = project.document().units.shown();
    let per_unit = shown.to_metres() as f32;
    let unit = numeric::Unit::from_scene(shown);
    let mut edited = current.clone();
    let mut committed = false;
    let dim = crate::theme::p().text_dim;

    ui.horizontal(|ui| {
        ui.label("Kind");
        let circle = matches!(current.kind, PatternKind::Circle { .. });
        for (label, wants_circle) in [("Line", false), ("Circle", true)] {
            if ui.selectable_label(circle == wants_circle, label).clicked() && circle != wants_circle {
                if let Some(start) = pattern::starting(project.document(), current.source, wants_circle) {
                    edited.kind = start.kind;
                    committed = true;
                }
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Count");
        let field = ui.add(egui::DragValue::new(&mut edited.count).range(2..=MOST).speed(0.1)).on_hover_text("How many there are, the original included");
        committed |= field.drag_stopped() || field.lost_focus();
    });
    let mut vector_row = |ui: &mut egui::Ui, label: &str, value: &mut Vec3, about: &str| {
        let mut shown_value = *value / per_unit;
        let before = shown_value;
        ui.horizontal(|ui| {
            ui.label(label).on_hover_text(about);
            for v in [&mut shown_value.x, &mut shown_value.y, &mut shown_value.z] {
                let field = ui.add(numeric::drag_value(v, unit).speed(0.01));
                committed |= field.drag_stopped() || field.lost_focus();
            }
        });
        if shown_value != before {
            *value += (shown_value - before) * per_unit;
        }
    };
    match &mut edited.kind {
        PatternKind::Line { step } => vector_row(ui, "Step", step, "From each copy to the next, along X, Y and Z"),
        PatternKind::Circle { centre, axis, angle } => {
            vector_row(ui, "Centre", centre, "The point the copies go round");
            ui.horizontal(|ui| {
                ui.label("Axis").on_hover_text("The line through the centre they turn about");
                for (label, direction) in [("X", Vec3::X), ("Y", Vec3::Y), ("Z", Vec3::Z)] {
                    let on = axis.try_normalize().is_some_and(|a| (a - direction).length() < 1e-3);
                    if ui.selectable_label(on, label).clicked() && !on {
                        *axis = direction;
                        committed = true;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Angle").on_hover_text("From the original to the last copy; 360° spaces them evenly all the way round");
                let mut degrees = angle.to_degrees();
                let field = ui.add(egui::DragValue::new(&mut degrees).range(1.0..=360.0).speed(0.5).suffix("°"));
                committed |= field.drag_stopped() || field.lost_focus();
                if (degrees - angle.to_degrees()).abs() > 1e-4 {
                    *angle = degrees.to_radians();
                }
            });
        }
    }

    if edited != current {
        let pattern = edited.clamped().with_ids(project.document(), Some(group));
        if let Err(error) = project.apply(Command::SetPattern { group, pattern: Some(Box::new(pattern)) }) {
            project.status = format!("Could not change the pattern: {error}");
        }
    }
    if committed {
        project.end_interaction();
    }

    // The copies are laid out from the original when the pattern changes;
    // moved since, it is out of step with them until laid out again.
    let stale = out_of_step(project, group);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(stale, egui::Button::new("Lay out again"))
            .on_hover_text("Put the copies back in step with the original, which has moved")
            .on_disabled_hover_text("The copies are in step with the original")
            .clicked()
        {
            if let Some(pattern) = project.document().tree.get(group).and_then(|g| g.pattern.clone()) {
                let _ = project.apply(Command::SetPattern { group, pattern: Some(Box::new(pattern.with_ids(project.document(), Some(group)))) });
                project.end_interaction();
            }
        }
        if ui.button("Make parts").on_hover_text("Keep the copies as parts of their own, and the pattern goes").clicked() {
            let _ = project.apply(Command::Ungroup { group });
        }
        if ui.button("Remove copies").on_hover_text("Take the copies away, and keep the original").clicked() {
            project.begin_group();
            let removed = project.apply(Command::SetPattern { group, pattern: None }).is_ok() && project.apply(Command::Ungroup { group }).is_ok();
            if removed {
                project.end_group();
                project.selection.select_many([current.source], SelectionMode::Replace);
            } else {
                project.abandon_group();
            }
        }
    });
    ui.label(egui::RichText::new("The copies share the original's mesh and material: a material on one is on all of them.").size(11.5).color(dim));
}

/// Whether the first copy is not where the pattern would put it now.
fn out_of_step(project: &Project, group: NodeId) -> bool {
    let document = project.document();
    let Some(pattern) = document.tree.get(group).and_then(|g| g.pattern.as_ref()) else { return false };
    let (Some(source), Some(first)) = (document.tree.get(pattern.source), pattern.copies.first().and_then(|c| document.tree.get(*c))) else {
        return false;
    };
    let wanted = pattern.offset(1) * source.transform.matrix;
    let probe = [Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)];
    probe.iter().any(|&p| (wanted.transform_point(p) - first.transform.point(p)).length() > 1e-5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (Project, NodeId) {
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.25);
        let project = Project::with_document(document, std::sync::Arc::new(tequila_scene::demo::studio_environment()));
        let part = project.document().tree.iter().find(|n| n.mesh().is_some()).unwrap().id;
        (project, part)
    }

    /// Made from the menu: the part grouped with four copies beside it, the
    /// group selected; one undo puts the part back as it was, ungrouped.
    #[test]
    fn a_pattern_made_from_a_part_is_one_step() {
        let (mut project, part) = project();
        let parent = project.document().tree.get(part).unwrap().parent;
        let nodes = project.document().tree.iter().count();
        project.selection.select_many([part], SelectionMode::Replace);
        assert_eq!(candidate(&project), Some(part));
        let group = make(&mut project, part, false).expect("a pattern");
        let made = project.document().tree.get(group).unwrap();
        assert_eq!(made.children.len(), 5, "the original and four copies");
        assert_eq!(made.children[0], part);
        assert_eq!(project.selection.nodes().collect::<Vec<_>>(), vec![group]);
        assert_eq!(candidate(&project), None, "a pattern is not patterned again");
        project.selection.select_many([part], SelectionMode::Replace);
        assert_eq!(candidate(&project), None, "nor is its original");
        assert!(!out_of_step(&project, group));

        project.undo();
        assert_eq!(project.document().tree.iter().count(), nodes, "one undo, all gone");
        assert_eq!(project.document().tree.get(part).unwrap().parent, parent);
    }

    /// Moving the original puts it out of step with its copies, until laid
    /// out again.
    #[test]
    fn a_moved_original_is_out_of_step() {
        let (mut project, part) = project();
        let group = make(&mut project, part, true).expect("a pattern");
        let moved = project.document().tree.get(part).unwrap().transform * tequila_core::math::Transform::translation(vec3(0.0, 0.1, 0.0));
        project.apply(Command::set_transform(part, moved)).unwrap();
        assert!(out_of_step(&project, group));
        let pattern = project.document().tree.get(group).unwrap().pattern.clone().unwrap();
        project.apply(Command::SetPattern { group, pattern: Some(Box::new(pattern.with_ids(project.document(), Some(group)))) }).unwrap();
        assert!(!out_of_step(&project, group));
    }
}
