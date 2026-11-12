//! Placing a light by pointing at the highlight you want.
//!
//! The HDRI Editor's defining gesture, and the reason pins beat a list of
//! numbers. Nobody arranging a product shot thinks "the key light belongs at
//! bearing 43 degrees, elevation 31" — they look at the render, decide the
//! highlight on the shoulder of the bottle should be *there*, and move the
//! light until it is. This is that, in one click: point at the surface, and the
//! pin goes wherever it must for its reflection to land where you pointed.
//!
//! The arithmetic is [`tequila_render_core::pins::direction_for_highlight`].
//! Everything here is the pointer handling around it, kept apart for the same
//! reason the label gizmo's is: the part that can be wrong should be testable
//! without an event loop.
//!
//! A light in the scene - a softbox near the product - is aimed the same
//! way: it moves to where the camera's line of sight bounces to off the
//! surface, as far from the point as it was, and turns to face it
//! ([`tequila_scene::studio_kit::aimed_at_highlight`]).

use crate::gizmo_ui;
use crate::orchestrator::Orchestrator;
use crate::project::Project;
use eframe::egui;
use tequila_render_core::camera::Camera;
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::Command;

/// What is being aimed: a pin on the environment, by its place in the
/// list, or a light in the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Pin(usize),
    Light(NodeId),
}

#[derive(Default)]
pub struct PinGizmo {
    target: Option<Target>,
}

impl PinGizmo {
    /// The pin being aimed, if one is.
    pub fn armed(&self) -> Option<usize> {
        match self.target {
            Some(Target::Pin(index)) => Some(index),
            _ => None,
        }
    }

    /// The light being aimed, if one is.
    pub fn armed_light(&self) -> Option<NodeId> {
        match self.target {
            Some(Target::Light(node)) => Some(node),
            _ => None,
        }
    }

    pub fn disarm(&mut self) {
        self.target = None;
    }

    /// Arm, or disarm if this same pin was already armed.
    pub fn toggle(&mut self, index: usize) {
        self.toggle_target(Target::Pin(index));
    }

    /// Arm for a light, or disarm if it was already armed.
    pub fn toggle_light(&mut self, node: NodeId) {
        self.toggle_target(Target::Light(node));
    }

    fn toggle_target(&mut self, target: Target) {
        if self.target == Some(target) {
            self.disarm();
        } else {
            self.target = Some(target);
        }
    }

    /// Take the primary click or drag, if armed. Returns whether it did.
    pub fn interact(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        camera: &Camera,
        project: &mut Project,
        orchestrator: &Orchestrator,
        response: &egui::Response,
    ) -> bool {
        let Some(target) = self.target else {
            return false;
        };
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.disarm();
            project.status = "Light placement finished".into();
            return true;
        }
        // The pin may have been removed while the mode was armed, in which
        // case this index now names a different light or none at all; and
        // a light may have been deleted.
        let gone = match target {
            Target::Pin(index) => index >= project.document().environment.pins.len(),
            Target::Light(node) => !matches!(project.document().tree.get(node).map(|n| &n.kind), Some(NodeKind::Light { .. })),
        };
        if gone {
            self.disarm();
            return false;
        }

        if response.drag_stopped() {
            project.end_interaction();
        }
        if !(response.clicked() || response.dragged_by(egui::PointerButton::Primary)) {
            return true;
        }
        let Some(at) = response.interact_pointer_pos() else {
            return true;
        };
        let Some(ray) = gizmo_ui::ray_through(camera, rect, at) else {
            return true;
        };
        let Some(hit) = orchestrator.pick(&ray) else {
            return true;
        };
        let command = match target {
            Target::Pin(index) => aim_command(project, index, hit.position, hit.shading_normal, camera),
            Target::Light(node) => aim_light_command(project, node, hit.position, hit.shading_normal, camera),
        };
        if let Some(command) = command {
            if let Err(error) = project.apply(command) {
                project.status = error.to_string();
            }
        }
        true
    }

    /// A line saying the mode is on, and which light is being aimed.
    ///
    /// Painted after the viewport image, like every other overlay: egui draws
    /// in call order within a layer and the image covers the viewport, so
    /// anything painted earlier is painted underneath it.
    pub fn draw(&self, ui: &egui::Ui, rect: egui::Rect, project: &Project) {
        let Some(target) = self.target else {
            return;
        };
        let name = match target {
            Target::Pin(index) => project.document().environment.pins.get(index).map(|pin| pin.name.clone()),
            Target::Light(node) => project.document().tree.get(node).map(|n| n.name.clone()),
        }
        .unwrap_or_default();
        crate::label_gizmo::banner(
            &ui.painter_at(rect),
            rect,
            &format!("Aiming “{name}” · click where you want the highlight · Esc to finish"),
        );
    }
}

/// The command that aims a pin at a point on the surface.
///
/// Outside the pointer handling, because this is the part that can be wrong:
/// the reflection, and remembering that the pin lives in the environment's
/// rotated space rather than the world's.
pub fn aim_command(
    project: &Project,
    index: usize,
    point: tequila_core::math::Vec3,
    normal: tequila_core::math::Vec3,
    camera: &Camera,
) -> Option<Command> {
    let settings = &project.document().environment;
    let direction = tequila_render_core::pins::direction_for_highlight(
        point,
        normal,
        camera.position(),
        settings.rotation,
    )?;
    let mut pins = settings.pins.clone();
    let pin = pins.get_mut(index)?;
    if pin.direction == direction {
        return None;
    }
    pin.direction = direction;
    Some(Command::SetEnvironmentPins { pins })
}

/// The command that moves the light `node` so its highlight lands on a
/// point on the surface: placed in the world, then put back in its parent's
/// space, since a light in a studio's group is placed relative to it.
pub fn aim_light_command(
    project: &Project,
    node: NodeId,
    point: tequila_core::math::Vec3,
    normal: tequila_core::math::Vec3,
    camera: &Camera,
) -> Option<Command> {
    let document = project.document();
    let scene_node = document.tree.get(node)?;
    let NodeKind::Light { settings } = &scene_node.kind else {
        return None;
    };
    let placed = document.placements();
    let world = placed.world(node);
    let aimed = tequila_scene::studio_kit::aimed_at_highlight(&world, &settings.shape, point, normal, camera.position())?;
    let local = match scene_node.parent {
        Some(parent) => tequila_core::math::Transform::from_matrix(placed.world(parent).inverse * aimed.matrix)?,
        None => aimed,
    };
    if local.matrix == scene_node.transform.matrix {
        return None;
    }
    Some(Command::SetTransform { transforms: vec![(node, local)] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec3, Vec3};
    use tequila_render_core::pins::Pin;

    fn project_with_a_pin(rotation: f32) -> Project {
        let mut project = Project::with_document(
            tequila_scene::document::SceneDocument::new(),
            std::sync::Arc::new(tequila_scene::demo::studio_environment()),
        );
        let _ = project.apply(Command::SetEnvironmentPins {
            pins: vec![Pin {
                name: "Key".into(),
                direction: vec3(0.0, 1.0, 0.0),
                ..Default::default()
            }],
        });
        if rotation != 0.0 {
            use tequila_scene::command::{EnvParam, EnvValue, EnvironmentParameter};
            let _ = project.apply(Command::SetEnvironmentParameter {
                parameter: EnvironmentParameter::new(
                    EnvParam::Rotation,
                    EnvValue::Scalar(rotation),
                ),
            });
        }
        project
    }

    fn camera_at(position: Vec3) -> Camera {
        Camera::look_at(position, Vec3::ZERO, Vec3::Y)
    }

    /// Clicking a point on the model moves the light so its reflection lands
    /// there — checked by mirroring back, not by restating the formula.
    #[test]
    fn aiming_puts_the_light_where_its_reflection_lands_on_the_click() {
        let project = project_with_a_pin(0.0);
        let camera = camera_at(vec3(0.0, 0.5, 4.0));
        let point = vec3(0.35, 0.3, 0.88);
        let normal = point.normalize();

        let command = aim_command(&project, 0, point, normal, &camera).expect("aims");
        let Command::SetEnvironmentPins { pins } = command else {
            panic!("aiming produced the wrong command");
        };

        let pin = pins[0].direction;
        let back = (normal * (2.0 * pin.dot(normal)) - pin).normalize();
        let to_camera = (camera.position() - point).normalize();
        assert!(
            (back - to_camera).length() < 1e-4,
            "the light at {pin:?} reflects to {back:?} rather than back to the \
             camera at {to_camera:?}"
        );
    }

    /// With the environment turned, the light has to be placed in the
    /// environment's space or the highlight lands off by that angle.
    #[test]
    fn aiming_accounts_for_the_environments_rotation() {
        let camera = camera_at(vec3(0.0, 0.5, 4.0));
        let point = vec3(0.35, 0.3, 0.88);
        let normal = point.normalize();

        let straight = aim_command(&project_with_a_pin(0.0), 0, point, normal, &camera);
        let turned = aim_command(&project_with_a_pin(1.1), 0, point, normal, &camera);

        let dir = |command: Option<Command>| match command.expect("aims") {
            Command::SetEnvironmentPins { pins } => pins[0].direction,
            _ => panic!("wrong command"),
        };
        assert!(
            (dir(straight) - dir(turned)).length() > 0.1,
            "turning the environment did not change where the light was put, \
             so the rotation is being ignored"
        );
    }

    /// Aiming a pin that is no longer there does nothing rather than moving
    /// whichever light took its index.
    #[test]
    fn aiming_a_missing_pin_does_nothing() {
        let project = project_with_a_pin(0.0);
        assert!(aim_command(
            &project,
            7,
            vec3(0.0, 0.0, 1.0),
            vec3(0.0, 0.0, 1.0),
            &camera_at(vec3(0.0, 0.0, 4.0))
        )
        .is_none());
    }

    /// Aiming where the light already points produces no command, so holding
    /// the pointer still does not fill the undo history.
    #[test]
    fn aiming_at_the_same_place_twice_produces_one_edit() {
        let mut project = project_with_a_pin(0.0);
        let camera = camera_at(vec3(0.0, 0.5, 4.0));
        let point = vec3(0.2, 0.1, 0.95);
        let normal = point.normalize();

        let first = aim_command(&project, 0, point, normal, &camera).expect("aims");
        project.apply(first).expect("applies");
        assert!(
            aim_command(&project, 0, point, normal, &camera).is_none(),
            "aiming at the same point again produced another edit"
        );
    }

    /// A degenerate click — the camera exactly on the surface — produces no
    /// command rather than a NaN direction the command bus would refuse.
    #[test]
    fn a_degenerate_click_produces_no_command() {
        let project = project_with_a_pin(0.0);
        let at = vec3(1.0, 2.0, 3.0);
        assert!(aim_command(&project, 0, at, vec3(0.0, 1.0, 0.0), &camera_at(at)).is_none());
    }

    /// A softbox in a group placed off the origin, aimed at a highlight,
    /// lands where its reflection off the surface goes to the camera, in
    /// the world, whatever its group's placement.
    #[test]
    fn a_light_in_a_group_aims_at_the_highlight_in_the_world() {
        use tequila_core::math::Transform;
        use tequila_scene::lighting::LightSettings;
        let mut project = Project::with_document(
            tequila_scene::document::SceneDocument::new(),
            std::sync::Arc::new(tequila_scene::demo::studio_environment()),
        );
        project.apply(Command::AddGroup { parent: None, name: "Studio lights".into() }).expect("a group");
        let group = project.document().tree.roots()[0];
        project.apply(Command::SetTransform { transforms: vec![(group, Transform::translation(vec3(0.5, 0.2, -0.3)))] }).expect("placed");
        project
            .apply(Command::AddLight { parent: Some(group), name: "Key".into(), settings: Box::new(LightSettings::default()) })
            .expect("a light");
        let light = project.document().tree.get(group).unwrap().children[0];
        project.apply(Command::SetTransform { transforms: vec![(light, Transform::translation(vec3(-1.0, 0.4, 0.6)))] }).expect("placed");
        let camera = camera_at(vec3(0.0, 0.5, 4.0));
        let point = vec3(0.1, 0.3, 0.9);
        let normal = point.normalize();
        let command = aim_light_command(&project, light, point, normal, &camera).expect("aims");
        project.apply(command).expect("applies");
        let world = project.document().placements().world(light);
        let at = world.point(Vec3::ZERO);
        let to_light = (at - point).normalize();
        let back = normal * (2.0 * normal.dot(to_light)) - to_light;
        assert!((back - (camera.position() - point).normalize()).length() < 1e-4, "its reflection goes to {back:?}");
        assert!((world.vector(Vec3::Z).normalize() + to_light).length() < 1e-4, "the panel does not face the point");
    }
}
