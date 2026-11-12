//! A photographer's kit, one click each (Lighting tab › Add to the studio):
//! a softbox, a strip light, a white reflector card and a black flag, each
//! sized to the product and put where a photographer would first stand it,
//! measured from the camera the picture is seen through.
//!
//! The softbox goes up to the camera's left as the key, the strip behind
//! on the right as a rim, the card low on the right to fill the shadow side
//! and the flag on the left beside the product, where it cuts the light
//! off that side and lays a black band down every glossy face that sees it
//! - what a studio does with a sheet of black foam board. The lights fall
//! off towards their edges as real ones do (`LightSettings::edge_falloff`);
//! the flag is a part, matte black, that the camera does not see but that
//! casts shadows and shows in reflections ([`crate::tree::Rays`]), since a
//! flag is something in the way of light and no light can be that.
//!
//! Each is one step to undo, and the light or the flag comes in selected,
//! ready to be moved or aimed at a highlight.

use crate::command::{reserve_keys, Command, ImportPayload};
use crate::document::SceneDocument;
use crate::lighting::{Intensity, LightSettings, LightShape};
use crate::material::MaterialDefinition;
use crate::tree::{NodeKind, Rays, SceneNode};
use tequila_core::math::{vec3, Mat4, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;

/// The pieces of the kit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    Softbox,
    Strip,
    Card,
    Flag,
}

impl Piece {
    pub const ALL: [Piece; 4] = [Piece::Softbox, Piece::Strip, Piece::Card, Piece::Flag];

    pub fn label(self) -> &'static str {
        match self {
            Piece::Softbox => "Softbox",
            Piece::Strip => "Strip light",
            Piece::Card => "Reflector card",
            Piece::Flag => "Flag",
        }
    }

    /// What it is for, in a photographer's words.
    pub fn description(self) -> &'static str {
        match self {
            Piece::Softbox => "A key light up to the camera's left, brighter in its middle than at its edges, as a real softbox is.",
            Piece::Strip => "A tall narrow light behind on the right: a bright line down the edges and along rounded corners.",
            Piece::Card => "A white card low on the right, bouncing a little light into the shadow side.",
            Piece::Flag => "A black card beside the product on the left: cuts the light off that side and lays a dark band down glossy faces. The camera never sees it.",
        }
    }

    /// Its name in a script.
    pub fn key(self) -> &'static str {
        match self {
            Piece::Softbox => "softbox",
            Piece::Strip => "strip",
            Piece::Card => "card",
            Piece::Flag => "flag",
        }
    }

    pub fn from_key(key: &str) -> Option<Piece> {
        Piece::ALL.into_iter().find(|p| p.key().eq_ignore_ascii_case(key.trim()))
    }

    /// Where it stands against the camera: degrees round from the camera's
    /// side of the product (0) to the camera's left (negative) or right,
    /// degrees up from the product's middle, and how far off, in the
    /// product's radii; then its width and height in radii.
    fn stance(self) -> (f32, f32, f32, f32, f32) {
        match self {
            Piece::Softbox => (-45.0, 35.0, 3.2, 1.6, 2.0),
            Piece::Strip => (140.0, 20.0, 3.0, 0.3, 2.6),
            Piece::Card => (70.0, 8.0, 2.6, 1.8, 1.8),
            Piece::Flag => (-100.0, 10.0, 2.4, 2.0, 2.6),
        }
    }
}

/// Where a piece goes and which way it faces, for a product whose bounds
/// have their middle at `middle` and a bounding sphere of `radius`, seen by
/// a camera at `eye`: a frame whose +Z faces the product's middle and
/// whose Y is as upright as that allows, at its place, and its width and
/// height in metres.
pub fn placement(piece: Piece, middle: Vec3, radius: f32, eye: Vec3) -> (Mat4, f32, f32) {
    let (round, up, off, width, height) = piece.stance();
    let radius = radius.max(1e-3);
    // The camera's bearing about the product, along the floor.
    let mut toward = vec3(eye.x - middle.x, 0.0, eye.z - middle.z);
    if toward.length() < 1e-6 {
        toward = Vec3::Z;
    }
    let toward = toward.normalize();
    // Turned round the vertical: negative to the camera's left, positive
    // to its right, which faces the product along -`toward`.
    let (sin, cos) = round.to_radians().sin_cos();
    let right = Vec3::Y.cross(toward);
    let flat = (toward * cos + right * sin).normalize();
    let (sin_up, cos_up) = up.to_radians().sin_cos();
    let out = flat * cos_up + Vec3::Y * sin_up;
    let position = middle + out * (off * radius);
    let z = -out;
    let x = if z.y.abs() > 0.999 { Vec3::X } else { Vec3::Y.cross(z).normalize() };
    let y = z.cross(x);
    (Mat4::translation(position) * Mat4::from_columns(x, y, z), width * radius, height * radius)
}

/// The commands that add `piece` to `document`, placed for the camera at
/// `eye`, to be applied together as one step, and the id of the node they
/// add: a light and its placement, or a flag, which comes placed.
pub fn add(document: &SceneDocument, piece: Piece, eye: Vec3) -> (Vec<Command>, crate::tree::NodeId) {
    let bounds = crate::snapshot::visible_bounds(document);
    let (middle, radius) = if bounds.is_empty() || !bounds.diagonal().x.is_finite() {
        (vec3(0.0, 0.05, 0.0), 0.1)
    } else {
        (bounds.center(), bounds.bounding_radius())
    };
    let (frame, width, height) = placement(piece, middle, radius, eye);
    let transform = Transform::from_matrix(frame).unwrap_or(Transform::IDENTITY);
    let (meshes, materials, ids) = reserve_keys(document, 1, 1, 1);
    let id = ids[0];
    let light = |nits: f32, falloff: f32| {
        let mut settings = LightSettings {
            shape: LightShape::Area { width, height, disc: false, two_sided: false },
            intensity: Intensity::Nits(nits),
            edge_falloff: falloff,
            ..LightSettings::default()
        };
        // Lights a studio stands out of the picture are not in it.
        settings.visibility.camera = false;
        settings
    };
    let settings = match piece {
        Piece::Softbox => light(10.0, 0.55),
        Piece::Strip => light(20.0, 0.45),
        Piece::Card => light(2.0, 0.2),
        Piece::Flag => {
            // A plane turned to face the product: the shape's plane faces
            // +Y, so its Y is the frame's Z and its depth the frame's Y.
            let mut node = SceneNode::new(id, piece.label(), NodeKind::Part { mesh: meshes[0] });
            let to_plane = Mat4::from_columns(Vec3::X, -Vec3::Z, Vec3::Y);
            node.transform = Transform::from_matrix(frame * to_plane).unwrap_or(Transform::IDENTITY);
            node.material = Some(materials[0]);
            node.rays = Rays { camera: false, ..Rays::default() };
            let mut spec = crate::shapes::ShapeSpec::of(crate::shapes::Shape::Plane, width);
            spec.set(crate::shapes::Dimension::Depth, height);
            node.shape = Some(spec);
            let mut black = MaterialDefinition::preset("Flag black", MaterialType::Plastic);
            {
                let material = black.material_mut();
                material.base_color = Spectrum::splat(0.01);
                material.specular_roughness = 0.9;
            }
            let payload = ImportPayload {
                meshes: vec![(meshes[0], spec.mesh())],
                textures: Vec::new(),
                materials: vec![(materials[0], black)],
                nodes: vec![node],
                roots: vec![id],
            };
            return (vec![Command::Import { payload: Box::new(payload) }], id);
        }
    };
    // The light takes the next node id, which is the one reserved.
    (
        vec![
            Command::AddLight { parent: None, name: piece.label().into(), settings: Box::new(settings) },
            Command::SetTransform { transforms: vec![(id, transform)] },
        ],
        id,
    )
}

/// Where a light goes so that its reflection lands on `point`, a surface
/// facing `normal`, seen from `eye`: the place the camera's line of sight
/// bounces to off the surface there, as far from the point as the light is
/// now, turned to face it - a panel's +Z, a spot's and a sun's -Z towards
/// it - and as large as it is. A photographer moves a light until the
/// highlight is where they want it; this is that in one click. `None`
/// where the eye is on the surface.
pub fn aimed_at_highlight(current: &Transform, shape: &LightShape, point: Vec3, normal: Vec3, eye: Vec3) -> Option<Transform> {
    let to_eye = (eye - point).try_normalize()?;
    let mut normal = normal.try_normalize()?;
    if normal.dot(to_eye) < 0.0 {
        normal = -normal;
    }
    let bounced = (normal * (2.0 * normal.dot(to_eye)) - to_eye).try_normalize()?;
    let matrix = current.matrix;
    let at = current.point(Vec3::ZERO);
    let distance = (at - point).length().max(1e-4);
    let position = point + bounced * distance;
    let scale = |axis: Vec3| matrix.transform_vector(axis).length().max(1e-9);
    let (sx, sy, sz) = (scale(Vec3::X), scale(Vec3::Y), scale(Vec3::Z));
    let z = match shape {
        LightShape::Area { .. } => -bounced,
        LightShape::Spot { .. } | LightShape::Sun { .. } => bounced,
        // A point shines every way: only where it is matters.
        LightShape::Point { .. } => {
            let mut moved = matrix;
            moved.m[0][3] = position.x;
            moved.m[1][3] = position.y;
            moved.m[2][3] = position.z;
            return Transform::from_matrix(moved);
        }
    };
    let x = if z.y.abs() > 0.999 { Vec3::X } else { Vec3::Y.cross(z).normalize() };
    let y = z.cross(x);
    Transform::from_matrix(Mat4::translation(position) * Mat4::from_columns(x * sx, y * sy, z * sz))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each piece faces the product's middle from where it is meant to be:
    /// the softbox up on the camera's left, the strip behind on the right,
    /// the card on the right, the flag on the left.
    #[test]
    fn each_piece_stands_where_a_photographer_would_put_it_facing_the_product() {
        let middle = vec3(0.0, 0.05, 0.0);
        let eye = vec3(0.0, 0.2, 1.0);
        for piece in Piece::ALL {
            let (frame, width, height) = placement(piece, middle, 0.1, eye);
            let position = frame.transform_point(Vec3::ZERO);
            let facing = frame.transform_vector(Vec3::Z).normalize();
            let to_middle = (middle - position).normalize();
            assert!((facing - to_middle).length() < 1e-4, "{piece:?} faces {facing:?}, not the product");
            assert!(width > 0.0 && height > 0.0);
            // Seen from the camera at +Z, its left is -X.
            match piece {
                Piece::Softbox => assert!(position.x < 0.0 && position.z > 0.0 && position.y > middle.y, "{position:?}"),
                Piece::Strip => assert!(position.x > 0.0 && position.z < 0.0, "{position:?}"),
                Piece::Card => assert!(position.x > 0.0, "{position:?}"),
                Piece::Flag => assert!(position.x < 0.0, "{position:?}"),
            }
        }
    }

    /// A product twice the size gets a kit twice the size, twice as far off.
    #[test]
    fn the_kit_grows_with_the_product() {
        let eye = vec3(0.3, 0.4, 2.0);
        let (small, w, h) = placement(Piece::Softbox, Vec3::ZERO, 0.1, eye);
        let (large, w2, h2) = placement(Piece::Softbox, Vec3::ZERO, 0.2, eye);
        let (p, p2) = (small.transform_point(Vec3::ZERO), large.transform_point(Vec3::ZERO));
        assert!((p2 - p * 2.0).length() < 1e-5 && (w2 - 2.0 * w).abs() < 1e-6 && (h2 - 2.0 * h).abs() < 1e-6);
    }

    /// Aimed at a highlight, a softbox's reflection off the surface comes
    /// back to the camera, the panel faces the point, and it keeps its
    /// distance and its size.
    #[test]
    fn a_light_aimed_at_a_highlight_mirrors_into_the_camera() {
        let current = Transform::from_matrix(Mat4::translation(vec3(-0.4, 0.3, 0.2)) * Mat4::scale(vec3(2.0, 2.0, 1.0))).unwrap();
        let point = vec3(0.02, 0.04, 0.03);
        let normal = vec3(0.3, 1.0, 0.2).normalize();
        let eye = vec3(0.5, 0.4, 0.8);
        let shape = LightShape::Area { width: 0.3, height: 0.2, disc: false, two_sided: false };
        let aimed = aimed_at_highlight(&current, &shape, point, normal, eye).expect("aims");
        let at = aimed.point(Vec3::ZERO);
        let to_light = (at - point).normalize();
        let back = normal * (2.0 * normal.dot(to_light)) - to_light;
        assert!((back - (eye - point).normalize()).length() < 1e-4, "the light's reflection goes to {back:?}");
        assert!(((at - point).length() - (current.point(Vec3::ZERO) - point).length()).abs() < 1e-5, "its distance changed");
        let facing = aimed.vector(Vec3::Z).normalize();
        assert!((facing + to_light).length() < 1e-4, "the panel faces {facing:?}");
        assert!((aimed.vector(Vec3::X).length() - 2.0).abs() < 1e-4, "its size changed");
        // A spot aims its -Z at the point.
        let spot = LightShape::Spot { inner_degrees: 20.0, outer_degrees: 30.0, falloff: 1.0 };
        let aimed = aimed_at_highlight(&current, &spot, point, normal, eye).expect("aims");
        assert!((aimed.vector(-Vec3::Z).normalize() + to_light).length() < 1e-4);
    }
}
