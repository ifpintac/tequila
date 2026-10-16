//! Placing a label by pointing at the model.
//!
//! The numbers in the Labels panel can put a label anywhere on a part, and
//! nobody wants to reach a logo's position by typing two offsets and watching
//! what happens. This is the other half: arm it, click the model, and the
//! artwork's centre lands under the cursor. Drag and it follows.
//!
//! ## Why it is a mode rather than always on
//!
//! Clicking in the viewport already means "select", and a placement that
//! competed with selection would make both unreliable. So placing is armed
//! from the label's own row, takes the primary button while it is armed, and
//! ends on Escape or when the button is pressed again — with the viewport
//! saying so the whole time, because a mode nobody can see is a mode that
//! eats the next click.
//!
//! Middle and right drags still navigate while armed. Placing a label is a
//! thing you do *while* looking around the part, and a mode that froze the
//! camera would have to be left and re-entered for every adjustment.

use crate::gizmo_ui;
use crate::orchestrator::Orchestrator;
use crate::project::Project;
use eframe::egui;
use tequila_render_core::camera::Camera;
use tequila_scene::{decal_fit, tree::MaterialKey, Command};

/// Which label is being placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub material: MaterialKey,
    /// Index into the material's decal list.
    pub index: usize,
}

#[derive(Default)]
pub struct LabelGizmo {
    target: Option<Target>,
}

impl LabelGizmo {
    pub fn arm(&mut self, target: Target) {
        self.target = Some(target);
    }

    pub fn disarm(&mut self) {
        self.target = None;
    }

    pub fn armed(&self) -> Option<Target> {
        self.target
    }

    /// Arm, or disarm if this same label was already armed.
    ///
    /// One button that turns the mode on and off, because a button that only
    /// arms leaves the user hunting for how to stop.
    pub fn toggle(&mut self, target: Target) {
        if self.target == Some(target) {
            self.disarm();
        } else {
            self.arm(target);
        }
    }

    /// Take the primary click or drag, if armed. Returns whether it did.
    ///
    /// The caller suppresses selection and orbiting for this frame when it
    /// returns true, in the same way it does for the transform gizmo.
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
            project.status = "Label placement finished".into();
            return true;
        }
        // The label may have been deleted, or its material deselected, while
        // the mode was armed. Placing into a list that no longer has that row
        // would either panic or move the wrong label.
        let still_there = project
            .document()
            .materials
            .get(target.material)
            .is_some_and(|m| target.index < m.graph.decals.len());
        if !still_there {
            self.disarm();
            return false;
        }

        if response.drag_stopped() {
            // One undo step for the whole drag, as a slider gets.
            project.end_interaction();
        }

        // Ctrl + scroll sizes the label, Shift + scroll turns it: the other
        // two handles of a Move Texture gizmo, on the wheel so the pointer
        // stays where the label is being looked at.
        if response.hovered() {
            let (scroll, modifiers) = ui.input(|i| (i.raw_scroll_delta.y, i.modifiers));
            if scroll != 0.0 && (modifiers.command || modifiers.shift) {
                self.adjust(project, target, scroll / 120.0, modifiers.shift);
                return true;
            }
        }
        let placing = response.clicked() || response.dragged_by(egui::PointerButton::Primary);
        if !placing {
            // Still consumed: armed means the primary button belongs to this,
            // so a press that hits nothing does not fall through to selection.
            return true;
        }
        let Some(at) = response.interact_pointer_pos() else {
            return true;
        };
        self.place(rect, camera, project, orchestrator, target, at);
        true
    }

    /// Whether a scroll with these modifiers is the label's rather than the
    /// camera's: sizing and turning while a label is being placed.
    pub fn takes_scroll(&self, modifiers: egui::Modifiers) -> bool {
        self.target.is_some() && (modifiers.command || modifiers.shift)
    }

    /// Size (or, with `turn`, rotate) the label by `steps` wheel notches.
    fn adjust(&mut self, project: &mut Project, target: Target, steps: f32, turn: bool) {
        let Some(definition) = project.document().materials.get(target.material) else { return };
        let mut decals = definition.graph.decals.clone();
        let Some(label) = decals.get_mut(target.index) else { return };
        if turn {
            label.mapping.rotation += steps * 5f32.to_radians();
        } else {
            let factor = (steps * 0.08).exp();
            label.mapping.scale.x = (label.mapping.scale.x * factor).clamp(1e-4, 1e4);
            label.mapping.scale.y = (label.mapping.scale.y * factor).clamp(1e-4, 1e4);
        }
        if let Err(error) = project.apply(Command::SetDecals { material: target.material, decals }) {
            project.status = error.to_string();
        }
    }

    fn place(
        &mut self,
        rect: egui::Rect,
        camera: &Camera,
        project: &mut Project,
        orchestrator: &Orchestrator,
        target: Target,
        at: egui::Pos2,
    ) {
        let Some(ray) = gizmo_ui::ray_through(camera, rect, at) else {
            return;
        };
        let Some(hit) = orchestrator.pick(&ray) else {
            return;
        };
        let Some(node) = project.mapping().node_of(hit.instance) else {
            return;
        };
        let Some(command) = placement_command(
            project.document(),
            target,
            node,
            hit.position,
            hit.shading_normal,
        ) else {
            return;
        };
        if let Err(error) = project.apply(command) {
            project.status = error.to_string();
        }
    }

    /// The artwork's frame on the part, and a line of instruction.
    ///
    /// Painted *after* the viewport image, like the transform gizmo: egui draws
    /// in call order within a layer, and the image covers the whole viewport,
    /// so anything painted earlier is painted underneath it and invisible.
    pub fn draw(&self, ui: &egui::Ui, rect: egui::Rect, camera: &Camera, project: &Project) {
        let Some(target) = self.target else {
            return;
        };
        let painter = ui.painter_at(rect);

        let document = project.document();
        if let Some(points) = document
            .materials
            .get(target.material)
            .and_then(|m| m.graph.decals.get(target.index))
            .zip(decal_fit::part_of(document, target.material))
            .and_then(|(decal, (node, bounds))| {
                let world = document.tree.world_transform(node);
                // 12 samples an edge: enough that a wrapped label's arc reads
                // as a curve rather than as a rectangle cutting through the
                // bottle, and few enough to be free.
                // The camera in the part's own space, so the frame is drawn
                // on the face turned towards the viewer.
                let viewer = world.inv_point(camera.position());
                let outline = decal_fit::outline(&decal.mapping, bounds, Some(viewer), 12)?;
                Some(
                    outline
                        .into_iter()
                        .map(|p| gizmo_ui::project_point(camera, rect, world.point(p)))
                        .collect::<Vec<_>>(),
                )
            })
        {
            let stroke = egui::Stroke::new(1.6_f32, egui::Color32::from_rgb(255, 196, 88));
            for i in 0..points.len() {
                // A segment is drawn only when both of its ends are in front of
                // the camera; one end behind it projects to a point on the
                // wrong side of the screen and would draw a line across the
                // whole viewport.
                if let (Some(a), Some(b)) = (points[i], points[(i + 1) % points.len()]) {
                    painter.line_segment([a, b], stroke);
                }
            }
        }

        let name = document
            .materials
            .get(target.material)
            .and_then(|m| m.graph.decals.get(target.index))
            .map(|d| d.name.clone())
            .unwrap_or_default();
        banner(&painter, rect, &format!("Placing “{name}” · click the part · Esc to finish"));
    }
}

/// The mode's own line, in the colour the frame is drawn in.
///
/// Bottom centre rather than a corner: the corners already carry the solid-mode
/// badge and the HUD, and a mode banner that shares a corner with a permanent
/// readout is one a user learns to stop seeing.
pub fn banner(painter: &egui::Painter, rect: egui::Rect, text: &str) {
    let galley = painter.layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(13.0),
        egui::Color32::from_rgb(28, 22, 8),
    );
    let position = egui::pos2(
        rect.center().x - galley.size().x * 0.5,
        rect.bottom() - galley.size().y - 22.0,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(
            position - egui::vec2(10.0, 5.0),
            galley.size() + egui::vec2(20.0, 10.0),
        ),
        6.0,
        egui::Color32::from_rgb(255, 196, 88),
    );
    painter.galley(position, galley, egui::Color32::PLACEHOLDER);
}

/// The command that moves a label to a point on a part, or `None` when there
/// is nothing to do.
///
/// Outside the pointer handling on purpose. Everything above this is egui and
/// a ray; everything in here is the part that can be wrong — which part was
/// hit, and getting the point out of world space and into the part's own.
pub fn placement_command(
    document: &tequila_scene::document::SceneDocument,
    target: Target,
    node: tequila_scene::tree::NodeId,
    world_point: tequila_core::math::Vec3,
    world_normal: tequila_core::math::Vec3,
) -> Option<Command> {
    // Only onto the part the label is actually on. Without this, dragging
    // across the backdrop or a neighbouring part throws the artwork somewhere
    // unrelated — the point would be read in the wrong object space, where it
    // means nothing.
    if document.tree.get(node).and_then(|n| n.material) != Some(target.material) {
        return None;
    }

    // Into the part's own object space, which is where the renderer shades and
    // therefore where a placement has to be expressed if the label is to stay
    // put when the part moves.
    let world = document.tree.world_transform(node);
    let point = world.inv_point(world_point);
    let normal = world.inv_normal(world_normal).normalize();

    let mut decals = document.materials.get(target.material)?.graph.decals.clone();
    let decal = decals.get_mut(target.index)?;
    let placed = decal_fit::place_at(decal.mapping, point, normal);
    if placed == decal.mapping {
        return None;
    }
    decal.mapping = placed;
    Some(Command::SetDecals {
        material: target.material,
        decals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3, Mat4, Transform, Vec3};
    use tequila_scene::document::SceneDocument;
    use tequila_scene::tree::{NodeId, NodeKind, SceneNode};
    use tequila_texture::projection::Projection;

    /// A plate wearing a label, at an arbitrary place in the world.
    ///
    /// The transform is the point of the fixture: a part sitting at the origin
    /// with no rotation makes world space and object space the same thing, and
    /// every conversion bug invisible.
    fn fitted() -> tequila_texture::projection::Mapping {
        tequila_scene::decal_fit::fit_to_bounds(tequila_core::math::Aabb {
            min: vec3(-2.0, 0.0, -2.0),
            max: vec3(2.0, 0.0, 2.0),
        })
        .expect("a plate can be fitted")
    }

    fn scene() -> (SceneDocument, NodeId, Target) {
        scene_with(fitted())
    }

    fn scene_with(
        mapping: tequila_texture::projection::Mapping,
    ) -> (SceneDocument, NodeId, Target) {
        let mut document = SceneDocument::new();
        let mesh = tequila_scene::geometry::Mesh::new(
            vec![
                vec3(-2.0, 0.0, -2.0),
                vec3(2.0, 0.0, -2.0),
                vec3(2.0, 0.0, 2.0),
                vec3(-2.0, 0.0, 2.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        );
        let (mesh_key, _) = document.geometry.insert_deduplicated(mesh);

        let material = document
            .materials
            .insert(tequila_scene::material::MaterialDefinition::new(
                "Shell",
                tequila_materials::material_type::MaterialType::Plastic,
                Default::default(),
            ));

        let node_id = NodeId(1);
        let mut node = SceneNode::new(node_id, "Plate", NodeKind::Part { mesh: mesh_key });
        node.material = Some(material);
        // Moved and turned, so object space and world space disagree. A part
        // at the origin with no rotation makes the two the same thing and
        // every conversion bug invisible.
        node.transform = Transform::from_matrix(
            Mat4::translation(vec3(7.0, 1.5, -3.0)) * Mat4::rotation(Vec3::Y, 0.6),
        )
        .expect("invertible");
        document
            .tree
            .restore(vec![node], vec![node_id])
            .expect("a one-node tree");

        // Added through the command, which is how a label comes into
        // existence: it interns the texture, and a decal referring to one the
        // document does not have is refused by every later edit.
        let mut bus = tequila_scene::CommandBus::new();
        bus.apply(
            &mut document,
            Command::AddDecal {
                material,
                source: tequila_scene::textures::TextureSource::new(
                    "logo.png",
                    tequila_texture::texture::MapRole::Color,
                ),
                name: "Logo".into(),
            },
        )
        .expect("the label is added");

        let target = Target { material, index: 0 };
        if decals_of(&document, target)[0].mapping != mapping {
            let mut decals = decals_of(&document, target);
            decals[0].mapping = mapping;
            bus.apply(&mut document, Command::SetDecals { material, decals })
                .expect("the placement is set");
        }
        (document, node_id, target)
    }

    fn decals_of(document: &SceneDocument, target: Target) -> Vec<tequila_materials::decal::Decal> {
        document
            .materials
            .get(target.material)
            .unwrap()
            .graph
            .decals
            .clone()
    }

    /// The claim: click a point on the part, and the artwork's centre is there.
    ///
    /// Stated in *world* space, because that is what the user pointed at, and
    /// checked by projecting the same world point back through the placed
    /// mapping after converting it the way the renderer does. A conversion
    /// dropped or applied the wrong way round fails here and nowhere else —
    /// the placement arithmetic itself is right either way.
    /// The wheel sizes and turns the label being placed, and only then does
    /// it take the wheel from the camera.
    #[test]
    fn the_wheel_sizes_and_turns_the_label_being_placed() {
        let (document, _, target) = scene();
        let mut project = crate::project::Project::with_document(
            document,
            std::sync::Arc::new(tequila_scene::demo::studio_environment()),
        );
        let before = decals_of(project.document(), target)[0].mapping;
        let mut gizmo = LabelGizmo::default();
        let ctrl = egui::Modifiers { command: true, ctrl: true, ..Default::default() };
        let shift = egui::Modifiers { shift: true, ..Default::default() };
        assert!(!gizmo.takes_scroll(ctrl), "the wheel was taken with no label being placed");
        gizmo.toggle(target);
        assert!(gizmo.takes_scroll(ctrl) && gizmo.takes_scroll(shift));
        assert!(!gizmo.takes_scroll(egui::Modifiers::default()), "a plain scroll must still zoom the camera");

        gizmo.adjust(&mut project, target, 2.0, false);
        let grown = decals_of(project.document(), target)[0].mapping;
        assert!(grown.scale.x > before.scale.x && grown.scale.y > before.scale.y, "scrolling up did not grow it");
        assert!(((grown.scale.x / before.scale.x) - (grown.scale.y / before.scale.y)).abs() < 1e-5, "it was stretched");

        gizmo.adjust(&mut project, target, -3.0, true);
        let turned = decals_of(project.document(), target)[0].mapping;
        assert!((turned.rotation - (before.rotation - 15f32.to_radians())).abs() < 1e-5);
    }

    #[test]
    fn a_click_puts_the_artwork_under_it_on_a_moved_part() {
        let (document, node, target) = scene();
        let world = document.tree.world_transform(node);

        // A point on the plate, in the part's own space, carried out to where
        // the user would have clicked it.
        for local in [
            vec3(1.2, 0.0, -0.7),
            vec3(-1.8, 0.0, 1.1),
            vec3(0.0, 0.0, 0.0),
        ] {
            let world_point = world.point(local);
            let world_normal = world.normal(vec3(0.0, 1.0, 0.0));

            let command =
                placement_command(&document, target, node, world_point, world_normal)
                    .expect("a point on the part places the label");
            let Command::SetDecals { decals, .. } = command else {
                panic!("placement produced the wrong command");
            };

            let mapping = decals[0].mapping;
            let coords = mapping.project(&tequila_texture::projection::SurfacePoint {
                position: local,
                normal: vec3(0.0, 1.0, 0.0),
                uv: vec2(0.0, 0.0),
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: tequila_core::spectrum::Spectrum::ONE,
                world: Default::default(),
            })[0]
                .coords;
            assert!(
                (coords.u - 0.5).abs() < 1e-3 && (coords.v - 0.5).abs() < 1e-3,
                "clicked {world_point:?} in the world, artwork centre landed at \
                 ({}, {})",
                coords.u,
                coords.v
            );
        }
    }

    /// A click on something else does nothing at all.
    ///
    /// Dragging a label across a part and off its edge onto the backdrop, or
    /// onto the bracket beside it, is an ordinary thing to do by accident. The
    /// point there is in a different part's object space and means nothing in
    /// this one, so the only right answer is to leave the label where it is.
    #[test]
    fn a_click_on_another_part_is_ignored() {
        let (mut document, _, target) = scene();

        let other_material =
            document
                .materials
                .insert(tequila_scene::material::MaterialDefinition::new(
                    "Bracket",
                    tequila_materials::material_type::MaterialType::Metal,
                    Default::default(),
                ));
        let mesh = document
            .tree
            .get(NodeId(1))
            .and_then(|n| n.mesh())
            .expect("the plate has a mesh");
        let other_id = NodeId(2);
        let mut other = SceneNode::new(other_id, "Bracket", NodeKind::Part { mesh });
        other.material = Some(other_material);
        let existing = document.tree.get(NodeId(1)).cloned().expect("the plate");
        document
            .tree
            .restore(vec![existing, other], vec![NodeId(1), other_id])
            .expect("a two-node tree");

        assert!(
            placement_command(
                &document,
                target,
                other_id,
                vec3(0.0, 0.0, 0.0),
                vec3(0.0, 1.0, 0.0)
            )
            .is_none(),
            "a click on a different part moved the label"
        );
    }

    /// A label whose row is gone produces nothing rather than moving whichever
    /// label happens to have taken its index.
    #[test]
    fn a_stale_target_places_nothing() {
        let (document, node, target) = scene();
        let stale = Target { index: 7, ..target };
        assert!(
            placement_command(&document, stale, node, vec3(7.0, 1.5, -3.0), Vec3::Y).is_none()
        );
    }

    /// Placing where the label already is produces no command, so holding the
    /// pointer still does not fill the undo history with identical steps.
    #[test]
    fn placing_a_label_where_it_already_is_does_nothing() {
        let (document, node, target) = scene();
        let world = document.tree.world_transform(node);
        let at = world.point(vec3(0.4, 0.0, -1.3));
        let normal = world.normal(vec3(0.0, 1.0, 0.0));

        let mut document = document;
        let command = placement_command(&document, target, node, at, normal).expect("places");
        // Applied the way the app applies it, through the bus, rather than by
        // reaching into the document — which the app cannot do either.
        tequila_scene::CommandBus::new()
            .apply(&mut document, command)
            .expect("the placement applies");

        assert!(
            placement_command(&document, target, node, at, normal).is_none(),
            "placing a label where it already sits produced another edit"
        );
    }

    /// A UV label has no frame to move, so pointing at the model leaves it
    /// alone rather than silently rewriting an authored layout.
    #[test]
    fn a_uv_label_is_not_moved_by_pointing_at_the_model() {
        let (document, node, target) = scene_with(Default::default());
        assert_eq!(
            decals_of(&document, target)[0].mapping.projection,
            Projection::Uv
        );

        let world = document.tree.world_transform(node);
        assert!(
            placement_command(
                &document,
                target,
                node,
                world.point(vec3(1.0, 0.0, 1.0)),
                world.normal(Vec3::Y)
            )
            .is_none(),
            "pointing at the model rewrote a UV placement"
        );
    }

    /// The button turns the mode off as well as on.
    #[test]
    fn the_place_button_toggles() {
        let target = Target {
            material: tequila_scene::tree::MaterialKey(3),
            index: 1,
        };
        let mut gizmo = LabelGizmo::default();
        assert_eq!(gizmo.armed(), None);
        gizmo.toggle(target);
        assert_eq!(gizmo.armed(), Some(target));
        gizmo.toggle(target);
        assert_eq!(gizmo.armed(), None, "the same button did not turn it off");

        // ...and asking for a different label moves the mode to it rather than
        // turning it off.
        let other = Target { index: 2, ..target };
        gizmo.toggle(target);
        gizmo.toggle(other);
        assert_eq!(gizmo.armed(), Some(other));
    }
}
