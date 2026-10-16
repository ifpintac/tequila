//! Orbit camera navigation.
//!
//! `TEQUILA_SPEC.md` §1.7 is explicit that these bindings must be replicated
//! exactly, and says why: "muscle memory is a real switching cost for this
//! audience". The full remappable hotkey system arrives with the preferences
//! panel; M0 implements the three that matter for a viewport — orbit, pan,
//! zoom — with the same bindings and the same feel.

use tequila_core::math::{vec3, Vec3};
use tequila_render_core::camera::{Camera, Projection};

/// A camera orbiting a target point.
///
/// Stored in spherical coordinates about the target rather than as a matrix,
/// because that is what makes orbiting stable: accumulating incremental
/// rotations into a matrix drifts, and the drift shows up as the horizon
/// slowly tilting after a few hundred drags.
#[derive(Clone, Copy, Debug)]
pub struct OrbitCamera {
    pub target: Vec3,
    /// Azimuth in radians, measured around `+Y`.
    pub yaw: f32,
    /// Elevation in radians, measured from the horizon.
    pub pitch: f32,
    pub distance: f32,
    pub camera: Camera,
}

/// How close to the pole the camera may get.
///
/// Exactly at the pole the up vector and the view direction are parallel and
/// the camera basis is undefined, which shows as the view snapping through a
/// half turn. Stopping just short is what every DCC application does.
const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 1e-3;

impl OrbitCamera {
    /// Frame a bounding sphere: the standard "fit the model in view" state a
    /// scene opens in.
    pub fn framing(center: Vec3, radius: f32, aspect: f32) -> Self {
        let mut camera = Camera::default();
        let fov = camera.vertical_fov(aspect).max(0.1);
        // Back off far enough that the sphere subtends slightly less than the
        // frame, with a margin so nothing touches the edge.
        let distance = (radius / (fov * 0.5).sin()) * 1.25;
        camera.focus_distance = distance;

        let mut orbit = OrbitCamera {
            target: center,
            yaw: 0.6,
            pitch: 0.25,
            distance: distance.max(1e-3),
            camera,
        };
        orbit.sync();
        orbit
    }

    /// Point at `target` from `distance`, keeping the direction the camera is
    /// already looking from.
    ///
    /// Keeping the direction is the whole difference between "frame this" and
    /// "reset the view". Reframing that also spun the camera back to a default
    /// three-quarter view would undo the shot somebody had just set up, which
    /// is the opposite of what pressing `F` is for.
    pub fn frame_at(&mut self, target: Vec3, distance: f32) {
        self.target = target;
        self.distance = distance.max(1e-3);
        self.camera.focus_distance = self.distance;
        self.sync();
    }

    /// Take over from a saved camera.
    ///
    /// The orbit parameters are recovered from the eye/target pair rather than
    /// kept alongside it, so there is one source of truth for where the camera
    /// is. A stored yaw/pitch/distance that disagreed with the matrix would
    /// show as the view jumping the first time it was orbited.
    pub fn adopt(&mut self, saved: &crate::lighting::SavedCamera) {
        self.camera = saved.camera;
        self.target = saved.target;
        let offset = saved.camera.position() - saved.target;
        self.distance = offset.length().max(1e-3);
        self.pitch = (offset.y / self.distance)
            .clamp(-1.0, 1.0)
            .asin()
            .clamp(-PITCH_LIMIT, PITCH_LIMIT);
        self.yaw = offset.x.atan2(offset.z);
        // Preserve the saved basis exactly. Rebuilding it through orbit angles
        // loses camera roll and introduces rounding edits on the next frame,
        // which can occupy the undo slot of the user's actual camera change.
    }

    /// The current view, as a camera a document can hold.
    pub fn to_saved(&self, name: impl Into<String>) -> crate::lighting::SavedCamera {
        crate::lighting::SavedCamera {
            name: name.into(),
            camera: self.camera,
            target: self.target,
        }
    }

    /// Rebuild the camera matrix from the orbit parameters.
    fn sync(&mut self) {
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let offset = vec3(cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw) * self.distance;
        let eye = self.target + offset;
        self.camera.camera_to_world = tequila_core::math::Mat4::look_at(eye, self.target, Vec3::Y);

        // "Keep Focus" (`docs/01` §8.1): re-derive the focus distance from the
        // camera-target distance whenever the camera moves, so that orbiting
        // around a subject does not drift it out of focus.
        if self.camera.keep_focus {
            self.camera.focus_distance = self.distance;
        }
    }

    /// Orbit. Bound to left-mouse drag.
    ///
    /// `delta` is in pixels; the scale is chosen so a drag across the viewport
    /// is roughly a half turn, which is the rate that feels neither sluggish
    /// nor twitchy.
    pub fn orbit(&mut self, delta: (f32, f32), viewport_height: f32) {
        let scale = std::f32::consts::PI / viewport_height.max(1.0);
        self.yaw -= delta.0 * scale;
        self.pitch = (self.pitch + delta.1 * scale).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        self.sync();
    }

    /// Pan. Bound to middle-mouse drag.
    ///
    /// The target moves in the camera's own screen plane, and the rate scales
    /// with distance so that a drag moves the model the same number of pixels
    /// regardless of how far away the camera is. Without that, panning a
    /// zoomed-out view feels stuck and a zoomed-in view feels uncontrollable.
    pub fn pan(&mut self, delta: (f32, f32), viewport_height: f32, aspect: f32) {
        let fov = self.camera.vertical_fov(aspect).max(1e-3);
        let world_per_pixel = 2.0 * self.distance * (fov * 0.5).tan() / viewport_height.max(1.0);
        let right = self.camera.camera_to_world.transform_vector(Vec3::X);
        let up = self.camera.camera_to_world.transform_vector(Vec3::Y);
        self.target += (right * -delta.0 + up * delta.1) * world_per_pixel;
        self.sync();
    }

    /// Zoom. Bound to the scroll wheel.
    ///
    /// Multiplicative, not additive: each notch changes the distance by a
    /// fixed *proportion*, so zooming feels the same at every scale. An
    /// additive step either crawls when far away or slams into the subject
    /// when close.
    pub fn zoom(&mut self, notches: f32) {
        let factor = (-notches * 0.12).exp();

        // An orthographic camera is framed by its *height*, not by how far
        // away it is: moving it changes nothing about what fills the frame,
        // because the rays are parallel. Scrolling therefore has to change the
        // scale.
        //
        // Doing the obvious thing — changing the distance, as for a
        // perspective camera — is what made orthographic look broken. The view
        // did not zoom at all, and pushing the camera far enough eventually
        // took it through the geometry and out past the environment, which is
        // the black.
        //
        // The distance still moves with it, because the camera's position
        // decides the *near plane*: an orthographic camera left close to the
        // subject clips it in half when the frame widens past it.
        if let Projection::Orthographic { scale } = &mut self.camera.projection {
            *scale = (*scale * factor).clamp(1e-4, 1e7);
            self.distance = (self.distance * factor).clamp(1e-3, 1e9);
            self.sync();
            return;
        }

        self.distance = (self.distance * factor).clamp(1e-3, 1e9);
        self.sync();
    }

    /// The world-space height the camera currently frames at its target.
    ///
    /// The bridge between the two projections: it is what an orthographic
    /// scale means directly, and what a perspective camera covers at its
    /// subject. Switching between them without it makes the view jump.
    pub fn framed_height(&self, aspect: f32) -> f32 {
        match self.camera.projection {
            Projection::Orthographic { scale } => scale,
            _ => 2.0 * self.distance * (self.camera.vertical_fov(aspect) * 0.5).tan(),
        }
    }

    /// Dolly the camera without changing the target. Bound to Alt + right-drag.
    pub fn dolly(&mut self, delta: f32) {
        self.zoom(-delta * 0.02);
    }

    /// Change the focal length. Bound to Alt + scroll (§1.7).
    pub fn change_focal_length(&mut self, notches: f32) {
        if let tequila_render_core::camera::Projection::Perspective { focal_length_mm } =
            &mut self.camera.projection
        {
            *focal_length_mm = (*focal_length_mm * (notches * 0.1).exp()).clamp(4.0, 800.0);
        }
    }

    /// Snap to one of the standard views, bound to Ctrl+Alt+1..7 (§1.7).
    /// Orbit `target` at the given angles (radians) and distance.
    pub fn place(&mut self, target: Vec3, yaw: f32, pitch: f32, distance: f32) {
        self.target = target;
        self.yaw = yaw;
        self.pitch = pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
        self.distance = distance.max(1e-3);
        self.sync();
    }

    /// Look at `target` from `eye`, as closely as an orbit can: straight up
    /// or down is nudged off the pole, where the view has no defined roll.
    pub fn place_from(&mut self, eye: Vec3, target: Vec3) {
        let offset = eye - target;
        let distance = offset.length().max(1e-3);
        let pitch = (offset.y / distance).clamp(-1.0, 1.0).asin();
        let yaw = offset.x.atan2(offset.z);
        self.place(target, yaw, pitch, distance);
    }

    /// A standard view by the name it is shown under, or `iso`.
    pub fn view_named(name: &str) -> Option<StandardView> {
        let name = name.trim().to_ascii_lowercase();
        if name == "iso" {
            return Some(StandardView::Isometric);
        }
        StandardView::ALL.into_iter().find(|v| v.label().eq_ignore_ascii_case(&name))
    }

    pub fn set_standard_view(&mut self, view: StandardView) {
        let (yaw, pitch) = view.angles();
        self.yaw = yaw;
        self.pitch = pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
        self.sync();
    }
}

/// The seven standard viewpoints of `TEQUILA_SPEC.md` §1.7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardView {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    Isometric,
}

impl StandardView {
    pub const ALL: [StandardView; 7] = [
        StandardView::Front,
        StandardView::Back,
        StandardView::Left,
        StandardView::Right,
        StandardView::Top,
        StandardView::Bottom,
        StandardView::Isometric,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StandardView::Front => "Front",
            StandardView::Back => "Back",
            StandardView::Left => "Left",
            StandardView::Right => "Right",
            StandardView::Top => "Top",
            StandardView::Bottom => "Bottom",
            StandardView::Isometric => "Isometric",
        }
    }

    /// The digit in `Ctrl`+`Alt`+`1..7`.
    pub fn digit(self) -> u32 {
        match self {
            StandardView::Front => 1,
            StandardView::Back => 2,
            StandardView::Left => 3,
            StandardView::Right => 4,
            StandardView::Top => 5,
            StandardView::Bottom => 6,
            StandardView::Isometric => 7,
        }
    }

    fn angles(self) -> (f32, f32) {
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        match self {
            StandardView::Front => (0.0, 0.0),
            StandardView::Back => (PI, 0.0),
            StandardView::Left => (-FRAC_PI_2, 0.0),
            StandardView::Right => (FRAC_PI_2, 0.0),
            StandardView::Top => (0.0, FRAC_PI_2),
            StandardView::Bottom => (0.0, -FRAC_PI_2),
            StandardView::Isometric => (FRAC_PI_4, 0.615), // atan(1/sqrt(2))
        }
    }

    pub fn from_digit(digit: u32) -> Option<Self> {
        Some(match digit {
            1 => StandardView::Front,
            2 => StandardView::Back,
            3 => StandardView::Left,
            4 => StandardView::Right,
            5 => StandardView::Top,
            6 => StandardView::Bottom,
            7 => StandardView::Isometric,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> OrbitCamera {
        OrbitCamera::framing(Vec3::ZERO, 1.0, 16.0 / 9.0)
    }

    /// Isometric means the three axes are equally foreshortened, and there is
    /// exactly one camera direction that does it: 45 degrees around, and
    /// `atan(1/sqrt 2)` — about 35.26 degrees — up. The test is the definition
    /// rather than the numbers, so it still holds if the numbers are ever
    /// rewritten.
    #[test]
    fn the_isometric_view_foreshortens_the_axes_equally() {
        let mut c = camera();
        c.set_standard_view(StandardView::Isometric);

        // Project each world axis onto the camera's screen plane and compare
        // the lengths. Equal lengths is what "isometric" means.
        let forward = c.camera.forward().normalize();
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward).normalize();
        let projected = |axis: Vec3| -> f32 {
            let x = axis.dot(right);
            let y = axis.dot(up);
            (x * x + y * y).sqrt()
        };
        let (a, b, c_) = (projected(Vec3::X), projected(Vec3::Y), projected(Vec3::Z));
        assert!(
            (a - b).abs() < 1e-3 && (b - c_).abs() < 1e-3,
            "axes project to {a}, {b}, {c_}; that is not isometric"
        );
    }

    /// Every standard view actually moves the camera somewhere different. A
    /// table with a duplicated row would leave one of them silently doing
    /// nothing, which is exactly how a broken view entry hides.
    #[test]
    fn the_seven_standard_views_are_seven_directions() {
        let mut seen: Vec<Vec3> = Vec::new();
        for view in StandardView::ALL {
            let mut c = camera();
            c.set_standard_view(view);
            let direction = c.camera.forward().normalize();
            for other in &seen {
                assert!(
                    (direction - *other).length() > 1e-2,
                    "{:?} points the same way as an earlier view",
                    view
                );
            }
            seen.push(direction);
        }
        assert_eq!(seen.len(), 7);
    }

    /// And the digits match the labels, since two lists in two files are how
    /// a menu comes to say "Isometric" and do "Bottom".
    #[test]
    fn digits_and_labels_agree() {
        for view in StandardView::ALL {
            assert_eq!(StandardView::from_digit(view.digit()), Some(view));
        }
    }

    /// Scrolling must change what an orthographic camera frames.
    ///
    /// It did not. Zoom moved the camera, and an orthographic camera's framing
    /// is its *height* — the rays are parallel, so where it sits changes
    /// nothing about what fills the frame. Scrolling appeared to do nothing at
    /// all, and pushing far enough took the camera through the geometry and out
    /// past the environment, which came out black.
    #[test]
    fn scrolling_zooms_an_orthographic_camera() {
        let mut c = camera();
        c.camera.projection = Projection::Orthographic { scale: 4.0 };

        c.zoom(1.0);
        let Projection::Orthographic { scale: closer } = c.camera.projection else {
            panic!("the projection changed");
        };
        assert!(closer < 4.0, "zooming in widened the frame to {closer}");

        c.zoom(-2.0);
        let Projection::Orthographic { scale: wider } = c.camera.projection else {
            panic!("the projection changed");
        };
        assert!(wider > 4.0, "zooming out narrowed the frame to {wider}");
    }

    /// And it keeps moving the camera as it does, because an orthographic
    /// camera's position still decides its near plane: one left close to the
    /// subject clips it in half as the frame widens past it.
    #[test]
    fn zooming_out_orthographically_also_backs_the_camera_off() {
        let mut c = camera();
        c.camera.projection = Projection::Orthographic { scale: 1.0 };
        let before = c.distance;
        c.zoom(-3.0);
        assert!(
            c.distance > before,
            "the camera stayed at {} while the frame widened",
            c.distance
        );
    }

    /// The height a camera frames is the same quantity under both projections,
    /// which is what lets one switch to the other without the view jumping.
    /// A perspective camera two units from its target with a 90 degree
    /// vertical field of view frames four units.
    #[test]
    fn the_framed_height_bridges_the_two_projections() {
        let mut c = camera();
        c.distance = 2.0;
        c.camera
            .set_vertical_fov(std::f32::consts::FRAC_PI_2, 1.0);
        let height = c.framed_height(1.0);
        assert!(
            (height - 4.0).abs() < 1e-3,
            "framed {height} rather than four units"
        );

        // And an orthographic camera reports its scale directly, so switching
        // across at that height changes nothing about what is on screen.
        c.camera.projection = Projection::Orthographic { scale: height };
        assert!((c.framed_height(1.0) - height).abs() < 1e-6);
    }

    /// Zoom is still zoom for a perspective camera: it moves the camera and
    /// leaves the lens alone.
    #[test]
    fn perspective_zoom_still_moves_the_camera() {
        let mut c = camera();
        let lens = c.camera.projection;
        let before = c.distance;
        c.zoom(1.0);
        assert!(c.distance < before);
        assert_eq!(c.camera.projection, lens, "zoom changed the lens");
    }

    #[test]
    fn framing_backs_off_far_enough_to_see_the_subject() {
        let c = camera();
        let distance = (c.camera.position() - c.target).length();
        assert!(distance > 1.0, "camera is inside the subject");
        // The subject must subtend most of the frame but not overflow it.
        let half_angle = (1.0f32 / distance).asin();
        let half_fov = c.camera.vertical_fov(16.0 / 9.0) * 0.5;
        assert!(half_angle < half_fov, "subject overflows the frame");
        assert!(half_angle > half_fov * 0.5, "subject is lost in the frame");
    }

    #[test]
    fn orbiting_preserves_the_distance_to_the_target() {
        let mut c = camera();
        let before = (c.camera.position() - c.target).length();
        for _ in 0..50 {
            c.orbit((13.0, 7.0), 800.0);
        }
        let after = (c.camera.position() - c.target).length();
        assert!(
            (before - after).abs() < 1e-3,
            "orbit drifted the distance: {before} -> {after}"
        );
    }

    /// Orbiting must not tumble through the pole. Storing the orientation as
    /// spherical angles rather than accumulating matrix rotations is what
    /// makes this hold.
    #[test]
    fn orbiting_stops_short_of_the_poles() {
        let mut c = camera();
        for _ in 0..500 {
            c.orbit((0.0, 50.0), 800.0);
        }
        assert!(c.pitch <= PITCH_LIMIT + 1e-6);
        // The camera must be above the target, and the frame must still be
        // well-defined rather than degenerate.
        assert!(c.camera.position().y > c.target.y);
        let forward = c.camera.forward();
        assert!(forward.is_finite() && (forward.length() - 1.0).abs() < 1e-4);

        for _ in 0..1000 {
            c.orbit((0.0, -50.0), 800.0);
        }
        assert!(c.pitch >= -PITCH_LIMIT - 1e-6);
        assert!(c.camera.forward().is_finite());
    }

    /// The horizon must not tilt. This is the failure mode of accumulating
    /// incremental rotations, and it takes hundreds of drags to notice.
    #[test]
    fn the_horizon_stays_level_after_many_orbits() {
        let mut c = camera();
        for i in 0..2000 {
            let d = ((i % 17) as f32 - 8.0, (i % 11) as f32 - 5.0);
            c.orbit(d, 800.0);
        }
        // The camera's right vector must stay horizontal.
        let right = c.camera.camera_to_world.transform_vector(Vec3::X);
        assert!(
            right.y.abs() < 1e-4,
            "the horizon tilted by {} after 2000 orbits",
            right.y
        );
    }

    /// Zoom must be multiplicative, so it feels identical at every scale.
    #[test]
    fn zoom_is_proportional() {
        let mut near = camera();
        near.distance = 1.0;
        near.sync();
        let mut far = camera();
        far.distance = 1000.0;
        far.sync();

        near.zoom(1.0);
        far.zoom(1.0);
        let near_ratio = near.distance / 1.0;
        let far_ratio = far.distance / 1000.0;
        assert!(
            (near_ratio - far_ratio).abs() < 1e-5,
            "zoom is not proportional: {near_ratio} vs {far_ratio}"
        );
        assert!(near_ratio < 1.0, "scrolling forward must move closer");
    }

    #[test]
    fn zoom_never_reaches_zero_or_infinity() {
        let mut c = camera();
        for _ in 0..10_000 {
            c.zoom(10.0);
        }
        assert!(c.distance > 0.0 && c.distance.is_finite());
        for _ in 0..20_000 {
            c.zoom(-10.0);
        }
        assert!(c.distance.is_finite());
    }

    /// Panning must move the subject by the same number of pixels regardless
    /// of distance, or it feels stuck when zoomed out and wild when zoomed in.
    #[test]
    fn pan_rate_is_independent_of_distance() {
        let aspect = 16.0 / 9.0;
        let measure = |distance: f32| {
            let mut c = camera();
            c.distance = distance;
            c.sync();
            let before = c.target;
            c.pan((100.0, 0.0), 800.0, aspect);
            let moved = (c.target - before).length();
            // Convert the world movement back to a screen angle.
            moved / distance
        };
        let near = measure(2.0);
        let far = measure(200.0);
        assert!(
            (near - far).abs() < 1e-5,
            "pan rate varies with distance: {near} vs {far}"
        );
    }

    #[test]
    fn pan_moves_in_the_screen_plane() {
        let mut c = camera();
        let forward = c.camera.forward();
        let before = c.target;
        c.pan((50.0, 30.0), 800.0, 1.5);
        let movement = c.target - before;
        assert!(movement.length() > 0.0);
        assert!(
            movement.dot(forward).abs() < 1e-4,
            "pan moved along the view direction"
        );
    }

    #[test]
    fn standard_views_look_from_the_expected_directions() {
        let expectations = [
            (StandardView::Front, vec3(0.0, 0.0, 1.0)),
            (StandardView::Back, vec3(0.0, 0.0, -1.0)),
            (StandardView::Right, vec3(1.0, 0.0, 0.0)),
            (StandardView::Left, vec3(-1.0, 0.0, 0.0)),
            (StandardView::Top, vec3(0.0, 1.0, 0.0)),
            (StandardView::Bottom, vec3(0.0, -1.0, 0.0)),
        ];
        for (view, expected) in expectations {
            let mut c = camera();
            c.set_standard_view(view);
            let direction = (c.camera.position() - c.target).normalize();
            assert!(
                (direction - expected).length() < 2e-3,
                "{view:?}: camera at {direction:?}, expected {expected:?}"
            );
            assert!(
                c.camera.forward().is_finite(),
                "{view:?} produced a degenerate view"
            );
        }
    }

    #[test]
    fn the_digit_bindings_cover_all_seven_views() {
        for d in 1..=7 {
            assert!(StandardView::from_digit(d).is_some(), "digit {d}");
        }
        assert!(StandardView::from_digit(0).is_none());
        assert!(StandardView::from_digit(8).is_none());
    }

    #[test]
    fn focal_length_changes_stay_in_a_usable_range() {
        let mut c = camera();
        for _ in 0..200 {
            c.change_focal_length(1.0);
        }
        if let tequila_render_core::camera::Projection::Perspective { focal_length_mm } =
            c.camera.projection
        {
            assert!((4.0..=800.0).contains(&focal_length_mm));
        }
        for _ in 0..400 {
            c.change_focal_length(-1.0);
        }
        if let tequila_render_core::camera::Projection::Perspective { focal_length_mm } =
            c.camera.projection
        {
            assert!((4.0..=800.0).contains(&focal_length_mm));
        }
    }

    /// Keep Focus (§8.1): with it enabled, orbiting must keep the subject in
    /// focus rather than letting it drift out.
    #[test]
    fn keep_focus_tracks_the_orbit_distance() {
        let mut c = camera();
        c.camera.keep_focus = true;
        c.zoom(3.0);
        assert!(
            (c.camera.focus_distance - c.distance).abs() < 1e-4,
            "focus did not follow the camera"
        );

        let mut without = camera();
        without.camera.keep_focus = false;
        let before = without.camera.focus_distance;
        without.zoom(3.0);
        assert_eq!(
            without.camera.focus_distance, before,
            "focus moved without Keep Focus enabled"
        );
    }

    #[test]
    fn adopting_a_saved_camera_preserves_its_exact_basis_and_roll() {
        let mut orbit = camera();
        let mut saved = orbit.to_saved("Scripted camera");
        saved.target = vec3(0.0, 0.62, 0.0);
        saved.camera.camera_to_world = tequila_core::math::Mat4::look_at(
            vec3(5.8, 1.9, 6.5), saved.target, vec3(0.2, 1.0, 0.1),
        );
        orbit.adopt(&saved);
        assert_eq!(orbit.camera, saved.camera);
        assert_eq!(orbit.target, saved.target);
        orbit.orbit((12.0, 8.0), 800.0);
        assert!(orbit.camera.position().is_finite());
        assert_ne!(orbit.camera.camera_to_world, saved.camera.camera_to_world);
    }
}
