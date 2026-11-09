//! The Shape section: a shape from Add › Shape set by number - a plane's
//! width and depth, a cylinder's diameter and height, a backdrop's width,
//! floor depth, wall height and curve (`tequila_scene::shapes`).

use crate::numeric;
use crate::project::Project;
use eframe::egui;
use tequila_scene::tree::NodeId;
use tequila_scene::Command;

/// The section for `node`, a part with a shape.
pub fn panel(ui: &mut egui::Ui, project: &mut Project, node: NodeId) {
    let Some(spec) = project.document().tree.get(node).and_then(|n| n.shape) else { return };
    let shown = project.document().units.shown();
    let per_unit = shown.to_metres() as f32;
    let unit = numeric::Unit::from_scene(shown);
    let mut edited = spec;
    let mut committed = false;
    ui.label(egui::RichText::new(spec.shape.label()).strong());
    for &(dimension, label) in spec.dimensions() {
        let mut value = spec.get(dimension) / per_unit;
        let before = value;
        ui.horizontal(|ui| {
            ui.label(label);
            let field = ui.add(numeric::drag_value(&mut value, unit).speed(0.01));
            committed |= field.drag_stopped() || field.lost_focus();
        });
        if value != before {
            edited.set(dimension, spec.get(dimension) + (value - before) * per_unit);
        }
    }
    if edited != spec {
        let (keys, _, _) = tequila_scene::command::reserve_keys(project.document(), 1, 0, 0);
        if let Err(error) = project.apply(Command::SetShape { node, shape: edited, mesh: keys[0] }) {
            project.status = format!("Could not change the shape: {error}");
        }
    }
    if committed {
        project.end_interaction();
    }
}

#[cfg(test)]
mod tests {
    use tequila_scene::shapes::{add, Dimension, Shape};
    use tequila_scene::Command;

    /// A backdrop added and given a deeper floor through the command the
    /// section issues is one step to undo.
    #[test]
    fn a_shape_resized_in_the_app_is_one_step() {
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.25);
        let mut project = crate::project::Project::with_document(document, std::sync::Arc::new(tequila_scene::demo::studio_environment()));
        let (payload, backdrop) = add(project.document(), Shape::Backdrop, None);
        project.apply(Command::Import { payload: Box::new(payload) }).unwrap();
        project.end_interaction();
        let start = project.document().tree.get(backdrop).unwrap().shape.unwrap();
        for step in 1..=4 {
            let mut deeper = start;
            deeper.set(Dimension::Depth, start.depth * (1.0 + step as f32 * 0.1));
            let (keys, _, _) = tequila_scene::command::reserve_keys(project.document(), 1, 0, 0);
            project.apply(Command::SetShape { node: backdrop, shape: deeper, mesh: keys[0] }).unwrap();
        }
        project.undo();
        assert_eq!(project.document().tree.get(backdrop).unwrap().shape, Some(start));
    }
}
