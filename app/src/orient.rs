//! Turning a model a quarter turn at a time, about its own middle, without
//! lifting it off the floor or sinking it in.
//!
//! Tequila is Y up, and a STEP file does not say which way up it was drawn:
//! Onshape, NX, CATIA and Fusion set to Z up write Z up, SolidWorks and
//! Fusion set to Y up write Y up. A Z-up model arrives lying on its back -
//! a keyboard standing on its edge - and Upright stands it the way it stood
//! in the CAD package. The other turns are for the rest: a front facing the
//! wrong way, a part drawn on its side.

use tequila_core::math::{vec3, Aabb, Mat4, Transform, Vec3};
use tequila_scene::tree::NodeId;
use tequila_scene::{Command, SceneDocument};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// Z up to Y up: the CAD package's top becomes the top.
    Upright,
    /// A quarter turn about the vertical, seen from above anticlockwise.
    AroundY,
    /// A quarter turn tipping the front up.
    AroundX,
    /// A quarter turn rolling it onto its side.
    AroundZ,
}

impl Turn {
    pub const ALL: [Turn; 4] = [Turn::Upright, Turn::AroundY, Turn::AroundX, Turn::AroundZ];

    pub fn label(self) -> &'static str {
        match self {
            Turn::Upright => "Upright (drawn Z up)",
            Turn::AroundY => "90° around the vertical",
            Turn::AroundX => "90° tipping forward",
            Turn::AroundZ => "90° onto its side",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Turn::Upright => {
                "For a CAD model lying on its back: most CAD packages draw Z up, Tequila is Y up. \
                 Stands it the way it stood in the CAD package."
            }
            Turn::AroundY => "Turn it to face another way, staying on the floor.",
            Turn::AroundX => "Tip it forward a quarter turn.",
            Turn::AroundZ => "Roll it onto its side a quarter turn.",
        }
    }

    fn rotation(self) -> Mat4 {
        let quarter = std::f32::consts::FRAC_PI_2;
        match self {
            // (x, y, z) to (x, z, -y): the file's +Z becomes +Y.
            Turn::Upright => Mat4::rotation(Vec3::X, -quarter),
            Turn::AroundY => Mat4::rotation(Vec3::Y, quarter),
            Turn::AroundX => Mat4::rotation(Vec3::X, quarter),
            Turn::AroundZ => Mat4::rotation(Vec3::Z, quarter),
        }
    }
}

/// The command turning `roots` together, about the middle of their bounds,
/// with their lowest point kept where it was: on the floor if it was on the
/// floor. One command, so one step to undo. `None` when there is nothing
/// with a size to turn.
pub fn turn(document: &SceneDocument, roots: &[NodeId], turn: Turn) -> Option<Command> {
    let mut bounds = Aabb::EMPTY;
    for &root in roots {
        bounds.extend(&document.tree.bounds(root, &|mesh| document.geometry.get(mesh).map(|m| m.bounds())));
    }
    if bounds.is_empty() || !(bounds.max - bounds.min).length().is_finite() {
        return None;
    }
    let centre = bounds.center();
    let about = Mat4::translation(centre) * turn.rotation() * Mat4::translation(-centre);
    // A quarter turn takes a box to a box, so the turned bounds are exact.
    let turned = bounds.transformed(&Transform::from_matrix(about)?);
    let about = Mat4::translation(vec3(0.0, bounds.min.y - turned.min.y, 0.0)) * about;
    let transforms = roots
        .iter()
        .filter_map(|&root| {
            let local = document.tree.get(root)?.transform;
            let world = document.tree.world_transform(root);
            // world = parent · local, so the new local is parent⁻¹ · about ·
            // world, and parent⁻¹ is local · world⁻¹.
            Transform::from_matrix(local.matrix * world.inverse * about * world.matrix).map(|t| (root, t))
        })
        .collect::<Vec<_>>();
    (!transforms.is_empty()).then_some(Command::SetTransform { transforms })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scene::command::{reserve_keys, ImportPayload};
    use tequila_scene::{CommandBus, MaterialDefinition, Mesh, NodeKind, SceneNode};

    /// A keyboard drawn Z up, 0.4 wide, 0.1 deep and 0.03 tall in the file,
    /// arrives 0.1 tall and 0.03 deep: standing on its edge, on the floor.
    fn keyboard() -> (SceneDocument, CommandBus, NodeId) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let corners: Vec<Vec3> =
            (0..8).map(|i| vec3(if i & 1 == 0 { 0.0 } else { 0.4 }, if i & 2 == 0 { 0.0 } else { 0.1 }, if i & 4 == 0 { 0.0 } else { 0.03 })).collect();
        let mesh = Mesh::new(corners, vec![0, 1, 3, 0, 3, 2, 4, 7, 5, 4, 6, 7]);
        let (meshes, materials, ids) = reserve_keys(&document, 1, 1, 2);
        // As the file had it, Z up, placed somewhere on the floor.
        let mut top = SceneNode::new(ids[0], "Model", NodeKind::Group);
        top.children = vec![ids[1]];
        top.transform = Transform::translation(vec3(1.0, 0.0, 2.0));
        let mut part = SceneNode::new(ids[1], "Keyboard", NodeKind::Part { mesh: meshes[0] });
        part.parent = Some(ids[0]);
        part.material = Some(materials[0]);
        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(meshes[0], mesh)],
                    textures: Vec::new(),
                    materials: vec![(materials[0], MaterialDefinition::plastic("Shell"))],
                    nodes: vec![top, part],
                    roots: vec![ids[0]],
                }),
            },
        )
        .unwrap();
        (document, bus, ids[0])
    }

    fn bounds(document: &SceneDocument) -> Aabb {
        document.bounds()
    }

    #[test]
    fn upright_stands_a_z_up_model_on_the_floor_where_it_was() {
        let (mut document, mut bus, top) = keyboard();
        let before = bounds(&document);
        assert!((before.min.y).abs() < 1e-5, "on the floor to begin with");
        let command = turn(&document, &[top], Turn::Upright).expect("there is a model to turn");
        bus.apply(&mut document, command).unwrap();
        let after = bounds(&document);
        let size = after.max - after.min;
        assert!((size.x - 0.4).abs() < 1e-4 && (size.y - 0.03).abs() < 1e-4 && (size.z - 0.1).abs() < 1e-4, "lying flat: {size:?}");
        assert!(after.min.y.abs() < 1e-5, "still on the floor: {}", after.min.y);
        let (c0, c1) = (before.center(), after.center());
        assert!((c0.x - c1.x).abs() < 1e-5 && (c0.z - c1.z).abs() < 1e-5, "where it was: {c0:?} {c1:?}");
        // One step to undo.
        bus.undo(&mut document).unwrap();
        let back = bounds(&document);
        assert!((back.max - back.min - (before.max - before.min)).length() < 1e-5);
    }

    /// The file's top is the top: what was at +Z ends up at +Y.
    #[test]
    fn upright_takes_the_files_top_to_the_top() {
        let up = Turn::Upright.rotation().transform_vector(Vec3::Z);
        assert!((up - Vec3::Y).length() < 1e-6, "{up:?}");
        let front = Turn::Upright.rotation().transform_vector(Vec3::Y);
        assert!((front - -Vec3::Z).length() < 1e-6, "{front:?}");
    }

    #[test]
    fn nothing_to_turn_is_no_command() {
        let document = SceneDocument::default();
        assert!(turn(&document, &[], Turn::AroundY).is_none());
    }
}
