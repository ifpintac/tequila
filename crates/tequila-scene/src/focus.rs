//! Focusing the camera on a point: the Camera tab's Pick focus, clicked in
//! the viewport, and a script's `look` with `focus` (`TEQUILA_SPEC.md` §1.7,
//! "click-to-focus").
//!
//! The one thing that can be wrong here is what "the distance to the point"
//! means. A thin lens is sharp on a *plane* square to its axis, not on a
//! sphere round the camera: [`Camera::generate_ray`] aims every lens sample
//! at `dir * (focus_distance / -dir.z)`, which lies on the plane
//! `z = -focus_distance` in the camera's frame, and Cycles does the same
//! (`Pfocus = D * (focaldistance / D.z)` in its perspective camera, which
//! `tequila-render-cycles` hands the focus distance unchanged). So the focus
//! distance of a point is its depth along the view axis. The straight-line
//! distance to it is longer for anything off the middle of the picture, and
//! focusing there would put the sharp plane behind the point that was
//! clicked: by a few per cent at the edge of a 50 mm frame, which f/1.4 on a
//! product shot shows. Blender's focus object is measured the same way.

use tequila_core::math::Vec3;
use tequila_render_core::camera::{Camera, Projection};

/// The aperture blur comes on at when focusing turns it on: the Camera tab's
/// own, when *Blur what is out of focus* is ticked.
pub const BLUR_F_NUMBER: f32 = 2.8;

/// The nearest a focus can be, in scene units, as [`Camera::look_at`]
/// keeps it: a point on the lens itself has no focus to speak of.
const NEAREST: f32 = 1e-4;

/// How far `point` is from `camera` as its focus distance measures it: along
/// the view axis, to the plane through the point square to it. `None` when
/// the point is behind the camera or on its lens, and for a 360° camera,
/// which has no axis.
///
/// Defined for an orthographic camera too, whose plane is the same, though
/// neither renderer blurs one.
pub fn focus_depth(camera: &Camera, point: Vec3) -> Option<f32> {
    if matches!(camera.projection, Projection::Equirectangular) {
        return None;
    }
    // In the camera's own frame, where it looks down -Z: the frame the
    // renderers' focus plane is set in, so a camera matrix with any scale
    // in it is measured as they measure it.
    let local = camera.camera_to_world.inverse()?.transform_point(point);
    let depth = -local.z;
    (depth.is_finite() && depth >= NEAREST).then_some(depth)
}

/// Whether the lens blurs at all, and so has a focus to pick: a perspective
/// or shift lens. Neither renderer blurs an orthographic or a 360° camera
/// (their aperture is zero), so a focus picked there would change nothing
/// anyone could see.
pub fn can_focus(projection: &Projection) -> bool {
    matches!(projection, Projection::Perspective { .. } | Projection::Shift { .. })
}

/// What focusing did, for the person to be told.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focused {
    /// The new focus distance, in scene units.
    pub distance: f32,
    /// The blur was off, so the focus meant nothing, and it is on now.
    pub blur_turned_on: bool,
    /// Keep focus was on. It sets the focus to the target's distance
    /// whenever the view moves, which would throw the picked point away on
    /// the first orbit, so it is off now.
    pub keep_focus_turned_off: bool,
}

/// Why a camera could not be focused on a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CannotFocus {
    /// An orthographic or 360° camera, which keeps everything sharp.
    NoBlur,
    /// The point is behind the camera, or on it.
    Behind,
}

impl CannotFocus {
    pub fn message(self) -> &'static str {
        match self {
            CannotFocus::NoBlur => "this camera is orthographic or 360°, which keep everything sharp, so there is no focus to set",
            CannotFocus::Behind => "that point is behind the camera",
        }
    }
}

/// Focus `camera` on `point`: the focus distance set to the point's depth
/// ([`focus_depth`]), the blur turned on if it was off, and Keep focus
/// turned off. The camera is left as it was when it cannot be.
pub fn focus_on(camera: &mut Camera, point: Vec3) -> Result<Focused, CannotFocus> {
    if !can_focus(&camera.projection) {
        return Err(CannotFocus::NoBlur);
    }
    let distance = focus_depth(camera, point).ok_or(CannotFocus::Behind)?;
    let blur_turned_on = !(camera.f_number.is_finite() && camera.f_number > 0.0);
    if blur_turned_on {
        camera.f_number = BLUR_F_NUMBER;
    }
    let keep_focus_turned_off = camera.keep_focus;
    camera.keep_focus = false;
    camera.focus_distance = distance;
    Ok(Focused { distance, blur_turned_on, keep_focus_turned_off })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec3, Mat4, Ray};
    use tequila_core::sampler::Sampler;

    const ASPECT: f32 = 1.5;

    /// How far `point` is from the line `ray` runs along.
    fn miss(ray: &Ray, point: Vec3) -> f32 {
        let to = point - ray.origin;
        (to - ray.direction * to.dot(ray.direction)).length()
    }

    /// The widest any of a lens's rays through `point`'s place on the film
    /// passes it: zero when the point is sharp.
    fn widest_miss(camera: &Camera, point: Vec3) -> f32 {
        let film = camera.project(point, ASPECT).expect("in front of the camera");
        (0..64)
            .map(|i| {
                let mut sampler = Sampler::new(i, i * 7, i, 3);
                miss(&camera.generate_ray(film, ASPECT, &mut sampler), point)
            })
            .fold(0.0, f32::max)
    }

    /// A camera at an angle, so the depth is not just a coordinate: up and
    /// off to the side, looking down at the origin.
    fn camera(projection: Projection) -> Camera {
        Camera {
            projection,
            f_number: 1.4,
            ..Camera::look_at(vec3(0.6, 0.35, 0.8), vec3(0.0, 0.05, 0.0), Vec3::Y)
        }
    }

    fn perspective() -> Projection {
        Projection::Perspective { focal_length_mm: 50.0 }
    }

    /// A point put a known way in front of the camera: `depth` along its
    /// axis and off to the side and up by `across` and `up`.
    fn placed(camera: &Camera, depth: f32, across: f32, up: f32) -> Vec3 {
        let right = camera.camera_to_world.transform_vector(Vec3::X).normalize();
        let above = camera.camera_to_world.transform_vector(Vec3::Y).normalize();
        camera.position() + camera.forward().normalize() * depth + right * across + above * up
    }

    /// The depth is measured along the view axis, whatever the point's
    /// place across the picture: a point off to the side is further away in
    /// a straight line than it is deep.
    #[test]
    fn the_depth_is_along_the_view_axis() {
        let camera = camera(perspective());
        let point = placed(&camera, 0.9, 0.2, -0.1);
        let depth = focus_depth(&camera, point).expect("in front");
        assert!((depth - 0.9).abs() < 1e-5, "depth {depth}, not 0.9");
        let straight = (point - camera.position()).length();
        assert!(straight > depth + 0.02, "the test point should be well off the axis ({straight} m away in a line)");
    }

    /// The point picked is sharp in Tequila's renderer: every ray across the
    /// lens through its place on the film passes through it. Focused at its
    /// straight-line distance instead, the rays miss it, so the test tells
    /// the two definitions apart.
    #[test]
    fn the_picked_point_is_sharp_through_every_part_of_the_lens() {
        for projection in [perspective(), Projection::Shift { focal_length_mm: 35.0, shift_x: 0.1, shift_y: 0.2 }] {
            let mut camera = camera(projection);
            let point = placed(&camera, 0.7, 0.18, 0.12);
            let focused = focus_on(&mut camera, point).expect("focuses");
            assert!((focused.distance - 0.7).abs() < 1e-5, "{projection:?}: focused at {}", focused.distance);
            let sharp = widest_miss(&camera, point);
            assert!(sharp < 1e-5, "{projection:?}: a lens ray passes {sharp} m from the picked point");

            camera.focus_distance = (point - camera.position()).length();
            let wrong = widest_miss(&camera, point);
            assert!(wrong > 20.0 * sharp.max(1e-6), "{projection:?}: focused along the ray, the rays still meet at the point ({wrong} m)");
        }
    }

    /// An orthographic camera's depth is the same plane's, though neither
    /// renderer blurs it, so focusing one is refused rather than setting a
    /// number that changes nothing.
    #[test]
    fn an_orthographic_camera_has_a_depth_but_no_focus_to_set() {
        let mut camera = camera(Projection::Orthographic { scale: 0.4 });
        let point = placed(&camera, 1.25, -0.1, 0.05);
        let depth = focus_depth(&camera, point).expect("in front");
        assert!((depth - 1.25).abs() < 1e-5, "depth {depth}");
        let before = camera;
        assert_eq!(focus_on(&mut camera, point), Err(CannotFocus::NoBlur));
        assert_eq!(camera, before, "a refusal leaves the camera alone");

        let mut panorama = Camera { projection: Projection::Equirectangular, ..before };
        assert_eq!(focus_depth(&panorama, point), None, "a 360° camera has no axis");
        assert_eq!(focus_on(&mut panorama, point), Err(CannotFocus::NoBlur));
    }

    /// A point behind the camera, or on its lens, has no focus; the camera
    /// is left as it was.
    #[test]
    fn a_point_behind_the_camera_changes_nothing() {
        let mut camera = camera(perspective());
        let before = camera;
        let behind = placed(&camera, -0.5, 0.0, 0.0);
        assert_eq!(focus_on(&mut camera, behind), Err(CannotFocus::Behind));
        let on_the_lens = camera.position();
        assert_eq!(focus_on(&mut camera, on_the_lens), Err(CannotFocus::Behind));
        assert_eq!(camera, before);
    }

    /// Focusing a camera that does not blur turns the blur on, at the Camera
    /// tab's f/2.8, and says so; one that blurs keeps its f-number. Keep
    /// focus is turned off either way, and said to be when it was on.
    #[test]
    fn focusing_turns_the_blur_on_and_keep_focus_off() {
        let mut sharp = Camera { f_number: f32::INFINITY, keep_focus: true, ..camera(perspective()) };
        let point = placed(&sharp, 0.5, 0.0, 0.0);
        let focused = focus_on(&mut sharp, point).unwrap();
        assert!(focused.blur_turned_on && focused.keep_focus_turned_off, "{focused:?}");
        assert_eq!(sharp.f_number, BLUR_F_NUMBER);
        assert!(!sharp.keep_focus);

        let mut blurred = Camera { f_number: 5.6, ..camera(perspective()) };
        let focused = focus_on(&mut blurred, point).unwrap();
        assert!(!focused.blur_turned_on && !focused.keep_focus_turned_off, "{focused:?}");
        assert_eq!(blurred.f_number, 5.6, "an f-number already set is kept");
    }

    /// A camera matrix with a scale in it is measured in the camera's own
    /// frame, as the renderers set its focus plane, not in the world's.
    #[test]
    fn a_scaled_camera_is_measured_in_its_own_frame() {
        let mut camera = camera(perspective());
        camera.camera_to_world = camera.camera_to_world * Mat4::scale(vec3(2.0, 2.0, 2.0));
        let point = camera.camera_to_world.transform_point(vec3(0.05, -0.02, -0.4));
        let depth = focus_depth(&camera, point).unwrap();
        assert!((depth - 0.4).abs() < 1e-5, "depth {depth} in the camera's frame");
        focus_on(&mut camera, point).unwrap();
        let film = camera.project(point, ASPECT).unwrap();
        let mut sampler = Sampler::new(1, 2, 3, 4);
        let ray = camera.generate_ray(film, ASPECT, &mut sampler);
        assert!(miss(&ray, point) < 1e-4, "{}", miss(&ray, point));
    }
}
