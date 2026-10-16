//! Cutaway volumes in the document (`docs/02` §3, "Cutaway").
//!
//! Two ways to cut, as `TEQUILA_SPEC.md` §1.11 describes them: a cutaway
//! *node* — a plane, box, sphere or cylinder placed with the gizmo like any
//! other node — and a part given the Cutaway material, whose own closed mesh
//! is the cutter. Either removes everything inside it from the render and
//! caps what it cuts through.
//!
//! The node's shape is the canonical one of
//! [`tequila_render_core::cutaway::CutterShape`], and its transform sizes
//! and places it: a box is the cube from -1 to 1, so its scale is its half
//! size. A plane removes what is in front of it, along its local +Z.

use tequila_core::math::{Aabb, Mat4, Transform, Vec3};
use tequila_core::spectrum::Spectrum;

/// The shape of a cutaway node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CutShape {
    /// Removes everything on one side: the classic section view.
    #[default]
    Plane,
    /// Removes a box: a corner cut out of a housing.
    Box,
    Sphere,
    /// About its local Z axis.
    Cylinder,
}

impl CutShape {
    pub const ALL: [CutShape; 4] = [CutShape::Plane, CutShape::Box, CutShape::Sphere, CutShape::Cylinder];

    pub fn label(self) -> &'static str {
        match self {
            CutShape::Plane => "Plane",
            CutShape::Box => "Box",
            CutShape::Sphere => "Sphere",
            CutShape::Cylinder => "Cylinder",
        }
    }

    /// As written to a file.
    pub fn key(self) -> &'static str {
        match self {
            CutShape::Plane => "plane",
            CutShape::Box => "box",
            CutShape::Sphere => "sphere",
            CutShape::Cylinder => "cylinder",
        }
    }

    pub fn from_key(key: &str) -> Option<CutShape> {
        CutShape::ALL.into_iter().find(|shape| shape.key() == key)
    }
}

/// What a cut face looks like.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CapStyle {
    /// The material of the part that was cut, so it reads as solid all the
    /// way through.
    Own,
    /// One colour for every cut face, the draughtsman's convention for
    /// telling a section from a surface at a glance.
    Color(Spectrum),
}

/// One cutaway node.
#[derive(Clone, Debug, PartialEq)]
pub struct CutawaySettings {
    pub shape: CutShape,
    pub cap: CapStyle,
    pub enabled: bool,
}

impl Default for CutawaySettings {
    fn default() -> Self {
        CutawaySettings {
            shape: CutShape::Plane,
            cap: CapStyle::Own,
            enabled: true,
        }
    }
}

/// Where a new cutter of this shape goes, so that it cuts something
/// visible straight away rather than arriving as a unit cube at the origin
/// of a model a metre across or a millimetre across.
///
/// `bounds` is what is to be cut and `toward_camera` the direction from it
/// to the viewer. Each shape takes the classic cut for it:
///
/// - a plane through the middle, removing the half nearest the camera — the
///   section view;
/// - a box from the middle out past the corner facing the camera — a corner
///   cut out of a housing;
/// - a sphere centred on the near face — a scoop out of the front;
/// - a cylinder bored through the middle along the view.
pub fn placement(shape: CutShape, bounds: Aabb, toward_camera: Vec3) -> Transform {
    let (center, half) = if bounds.is_empty() {
        (Vec3::ZERO, Vec3::ONE)
    } else {
        (bounds.center(), (bounds.diagonal() * 0.5).max(Vec3::splat(1e-3)))
    };
    let radius = half.length().max(1e-3);
    let toward = toward_camera.try_normalize().unwrap_or(Vec3::Z);
    // A frame whose +Z is `toward`.
    let facing = |z: Vec3| {
        let reference = if z.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
        let x = reference.cross(z).normalize();
        let y = z.cross(x);
        Mat4::from_columns(x, y, z)
    };
    let matrix = match shape {
        CutShape::Plane => {
            Mat4::translation(center) * facing(toward) * Mat4::scale(Vec3::splat(radius))
        }
        CutShape::Box => {
            // The octant nearest the camera, with the top rather than the
            // bottom when the view is level: the cut is seen from above.
            let sign = |v: f32| if v < 0.0 { -1.0 } else { 1.0 };
            let octant = Vec3 {
                x: sign(toward.x),
                y: sign(toward.y + 1e-3),
                z: sign(toward.z),
            };
            let size = half * 0.55;
            Mat4::translation(center + octant * size) * Mat4::scale(size)
        }
        CutShape::Sphere => {
            let at = center + toward * (half * toward.abs()).length();
            Mat4::translation(at) * Mat4::scale(Vec3::splat(radius * 0.45))
        }
        CutShape::Cylinder => {
            Mat4::translation(center)
                * facing(toward)
                * Mat4::scale(Vec3 {
                    x: radius * 0.25,
                    y: radius * 0.25,
                    z: radius * 1.2,
                })
        }
    };
    Transform::from_matrix(matrix).unwrap_or(Transform::IDENTITY)
}

impl CutawaySettings {
    /// The renderer's shape for this node.
    pub fn render_shape(&self) -> tequila_render_core::cutaway::CutterShape {
        use tequila_render_core::cutaway::CutterShape;
        match self.shape {
            CutShape::Plane => CutterShape::Plane,
            CutShape::Box => CutterShape::Box,
            CutShape::Sphere => CutterShape::Sphere,
            CutShape::Cylinder => CutterShape::Cylinder,
        }
    }

    /// Whether a colour and values are all numbers the renderer can use.
    pub fn is_valid(&self) -> bool {
        match self.cap {
            CapStyle::Own => true,
            CapStyle::Color(c) => c.r.is_finite() && c.g.is_finite() && c.b.is_finite(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;
    use tequila_render_core::cutaway::Cutter;

    fn cutter(shape: CutShape, transform: Transform) -> Cutter {
        let settings = CutawaySettings {
            shape,
            ..Default::default()
        };
        Cutter::new(settings.render_shape(), transform)
    }

    /// Each default placement cuts the model where the camera can see it,
    /// and leaves the far side alone — at any scale.
    #[test]
    fn a_new_cutter_cuts_the_near_side_and_not_the_far() {
        for scale in [0.01f32, 1.0, 400.0] {
            let bounds = Aabb::from_points(vec3(-1.0, -0.5, -0.8) * scale, vec3(1.0, 0.5, 0.8) * scale);
            let toward = vec3(0.3, 0.4, 1.0);
            let center = bounds.center();
            let near = center + vec3(0.25, 0.2, 0.7) * scale;
            let far = center - vec3(0.25, 0.2, 0.7) * scale;
            // Off the line of sight through the middle, for the cylinder,
            // which bores straight through along it by design.
            let aside = center + toward.cross(Vec3::Y).normalize() * 0.6 * scale;
            for shape in CutShape::ALL {
                let c = cutter(shape, placement(shape, bounds, toward));
                let (kept, cut) = if shape == CutShape::Cylinder { (aside, center) } else { (far, near) };
                assert!(!c.contains(kept), "{shape:?} at scale {scale} cut what it should have left");
                assert!(c.contains(cut), "{shape:?} at scale {scale} does not cut where the camera looks");
            }
        }
    }

    /// A plane faces the camera, whatever the view: it removes the side the
    /// viewer is on.
    #[test]
    fn a_plane_removes_the_side_the_viewer_is_on() {
        let bounds = Aabb::from_points(Vec3::splat(-1.0), Vec3::splat(1.0));
        for toward in [Vec3::X, -Vec3::Z, Vec3::Y, vec3(-0.5, 0.2, -0.8)] {
            let c = cutter(CutShape::Plane, placement(CutShape::Plane, bounds, toward));
            let toward = toward.normalize();
            assert!(c.contains(toward * 0.5), "looking from {toward:?} the near side stayed");
            assert!(!c.contains(-toward * 0.5), "looking from {toward:?} the far side went");
        }
    }
}
