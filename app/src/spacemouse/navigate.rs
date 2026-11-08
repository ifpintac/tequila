//! The six axes turned into the orbit camera's moves, the way CAD
//! applications move the view with a 3D mouse: the cap moves the model.
//! Push it right and the model goes right; lift it and the model rises;
//! pull it toward you and the model comes closer; twist it and the model
//! turns about the vertical, tilt it forward and the model tips away.
//!
//! Every move is a rate - so much a second at full deflection - taken over
//! the frame's time, so the view moves alike at 30 frames a second and at
//! 144. Panning goes by the height the view frames at the model, and
//! zooming by a share of the distance, so both feel the same near and far.
//! Turning is about a pivot - the selection's centre, or the model's - and
//! the camera turns about it without the view jumping to face it.
//!
//! The orbit camera keeps the horizon level, as the mouse's orbit does: the
//! roll axis is read and shown, and moves nothing.

use crate::orbit::OrbitCamera;
use tequila_core::math::{Mat4, Vec3};

/// The six axes in the order the settings name them.
pub const AXES: [&str; 6] = ["Pan left and right", "Pan up and down", "Zoom", "Tilt", "Spin", "Roll"];

/// What a button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonAction {
    Nothing,
    /// Frame the selection, or everything when nothing is selected (`F`).
    FitView,
    /// Apply the next Studio in the list, round to the first again; with no
    /// Studios, look through the next saved camera.
    NextStudio,
    PreviousStudio,
}

impl ButtonAction {
    pub const ALL: [ButtonAction; 4] = [ButtonAction::FitView, ButtonAction::NextStudio, ButtonAction::PreviousStudio, ButtonAction::Nothing];

    pub fn label(self) -> &'static str {
        match self {
            ButtonAction::Nothing => "Nothing",
            ButtonAction::FitView => "Fit view",
            ButtonAction::NextStudio => "Next Snapshot",
            ButtonAction::PreviousStudio => "Previous Snapshot",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ButtonAction::Nothing => "nothing",
            ButtonAction::FitView => "fit",
            ButtonAction::NextStudio => "next_studio",
            ButtonAction::PreviousStudio => "previous_studio",
        }
    }

    pub fn from_key(key: &str) -> Option<ButtonAction> {
        ButtonAction::ALL.into_iter().find(|a| a.key() == key)
    }
}

/// What the view turns about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pivot {
    /// The selection's centre, or the model's when nothing is selected.
    Model,
    /// The point the view orbits, as the mouse's orbit does.
    View,
}

/// The person's choices, kept in the preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    /// How fast, 1 the usual: every rate is multiplied by it.
    pub sensitivity: f32,
    /// The share of each axis's travel, from the middle, that does nothing:
    /// a cap resting a little off centre leaves the view alone.
    pub dead_zone: f32,
    /// Each axis turned round, in the order of [`AXES`].
    pub invert: [bool; 6],
    /// Only the axis pushed furthest moves the view, for a straight pan or
    /// a pure turn.
    pub dominant: bool,
    pub pivot: Pivot,
    /// The left button (Menu on the larger devices) and the right (Fit).
    pub buttons: [ButtonAction; 2],
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            enabled: true,
            sensitivity: 1.0,
            dead_zone: 0.06,
            invert: [false; 6],
            dominant: false,
            pivot: Pivot::Model,
            buttons: [ButtonAction::NextStudio, ButtonAction::FitView],
        }
    }
}

/// At full deflection and a sensitivity of 1: radians a second of turn,
/// framed heights a second of pan, and the natural log of how much the
/// distance changes in a second of zoom.
pub const TURN_RATE: f32 = 1.6;
pub const PAN_RATE: f32 = 1.2;
pub const ZOOM_RATE: f32 = 1.5;

/// How long the axes take to catch up with the device: enough to smooth
/// reports that come 60 times a second into a view drawn at 144, too short
/// to feel late.
pub const SMOOTHING: f32 = 0.05;

impl Settings {
    /// The device's axes as the view should take them: inverted where
    /// asked, the dead zone taken out of the middle of each, a gentle curve
    /// for fine control near the middle, and only the strongest when asked.
    /// Full deflection still gives 1.
    pub fn shape(&self, axes: [f32; 6]) -> [f32; 6] {
        let dead = self.dead_zone.clamp(0.0, 0.9);
        let mut out = [0.0; 6];
        for (i, value) in axes.iter().enumerate() {
            let v = if self.invert[i] { -value } else { *value };
            let beyond = ((v.abs() - dead) / (1.0 - dead)).max(0.0);
            // Half the speed at half way, all of it at the end.
            out[i] = v.signum() * beyond * (0.35 + 0.65 * beyond.min(1.5));
        }
        if self.dominant {
            let strongest = (0..6).max_by(|&a, &b| out[a].abs().total_cmp(&out[b].abs())).unwrap_or(0);
            for (i, v) in out.iter_mut().enumerate() {
                if i != strongest {
                    *v = 0.0;
                }
            }
        }
        out
    }
}

/// Move `camera` by `axes` (already shaped) over `dt` seconds, turning about
/// `pivot`. `aspect` is the view's width over its height. True when the
/// view moved.
pub fn drive(camera: &mut OrbitCamera, axes: [f32; 6], dt: f32, aspect: f32, pivot: Vec3, sensitivity: f32) -> bool {
    if axes.iter().all(|v| *v == 0.0) || dt <= 0.0 {
        return false;
    }
    let k = sensitivity.clamp(0.05, 10.0) * dt;
    let [tx, ty, tz, rx, ry, _roll] = axes;

    // Turning: the model spins by `ry` about the vertical and tips by `rx`
    // about the view's horizontal, so the camera goes the other way round
    // the pivot. Tipping the top away shows less of it: the camera drops.
    if rx != 0.0 || ry != 0.0 {
        turn_about(camera, pivot, -ry * TURN_RATE * k, rx * TURN_RATE * k);
    }

    // Panning, by the height the view frames at the pivot: the model goes
    // the way the cap goes, so the camera's target goes the other.
    if tx != 0.0 || ty != 0.0 {
        let right = camera.camera.camera_to_world.transform_vector(Vec3::X);
        let up = camera.camera.camera_to_world.transform_vector(Vec3::Y);
        let forward = camera.camera.camera_to_world.transform_vector(-Vec3::Z);
        let depth = (pivot - camera.camera.position()).dot(forward).max(camera.distance * 0.05).max(1e-4);
        let height = match camera.camera.projection {
            tequila_render_core::camera::Projection::Orthographic { scale } => scale,
            _ => 2.0 * depth * (camera.camera.vertical_fov(aspect) * 0.5).tan(),
        };
        let step = height * PAN_RATE * k;
        let target = camera.target - right * (tx * step) - up * (ty * step);
        let (yaw, pitch, distance) = (camera.yaw, camera.pitch, camera.distance);
        camera.place(target, yaw, pitch, distance);
    }

    // Zooming: pulled toward the person, the model comes closer. A share
    // of the distance a second, as the wheel's zoom is a share a notch.
    if tz != 0.0 {
        camera.zoom(tz * ZOOM_RATE * k / ZOOM_PER_NOTCH);
    }
    true
}

/// The natural log of [`OrbitCamera::zoom`]'s factor for one notch.
const ZOOM_PER_NOTCH: f32 = 0.12;

/// Turn the camera about `pivot`: `yaw` radians about the vertical through
/// it, and `pitch` radians up over it (as far as the orbit allows). The
/// target turns with the eye, so the view keeps its direction relative to
/// the turn and nothing jumps when the pivot is not where it looks.
pub fn turn_about(camera: &mut OrbitCamera, pivot: Vec3, yaw: f32, pitch: f32) {
    let limit = std::f32::consts::FRAC_PI_2 - 1e-3;
    let pitch = (camera.pitch + pitch).clamp(-limit, limit) - camera.pitch;
    let spin = Mat4::rotation(Vec3::Y, yaw);
    let mut target = pivot + spin.transform_vector(camera.target - pivot);
    // The right axis after the spin: raising the eye turns the offset to
    // it about the right by minus the angle (the offset looks backwards).
    let right = spin.transform_vector(camera.camera.camera_to_world.transform_vector(Vec3::X));
    if pitch != 0.0 && right.length() > 1e-6 {
        let tip = Mat4::rotation(right, -pitch);
        target = pivot + tip.transform_vector(target - pivot);
    }
    let distance = camera.distance;
    let (new_yaw, new_pitch) = (camera.yaw + yaw, camera.pitch + pitch);
    camera.place(target, new_yaw, new_pitch, distance);
}

/// The device's axes, followed smoothly: each frame they move toward what
/// the device last said by a share set by the frame's time, and the camera
/// moves by their mean over the frame - exactly, for a device holding
/// still between reports - so the view moves alike however often frames
/// come.
#[derive(Clone, Copy, Debug, Default)]
pub struct Smoothed {
    /// Where they are now.
    pub axes: [f32; 6],
    /// Their mean over the frame just followed: what to move by.
    pub mean: [f32; 6],
}

impl Smoothed {
    /// Move toward `target` over `dt` seconds; true while anything is
    /// still moving.
    pub fn follow(&mut self, target: [f32; 6], dt: f32) -> bool {
        let dt = dt.max(0.0);
        let left = (-dt / SMOOTHING).exp();
        // The mean of an exponential approach over the frame.
        let carried = if dt > 0.0 { SMOOTHING / dt * (1.0 - left) } else { 1.0 };
        let mut moving = false;
        for ((axis, mean), goal) in self.axes.iter_mut().zip(self.mean.iter_mut()).zip(target) {
            *mean = goal + (*axis - goal) * carried;
            *axis = goal + (*axis - goal) * left;
            if goal == 0.0 && axis.abs() < 2e-3 {
                *axis = 0.0;
                if mean.abs() < 2e-3 {
                    *mean = 0.0;
                }
            }
            moving |= *axis != 0.0 || *mean != 0.0;
        }
        moving
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    fn camera() -> OrbitCamera {
        OrbitCamera::framing(vec3(0.0, 0.0, 0.0), 1.0, 1.5)
    }

    fn eye(c: &OrbitCamera) -> Vec3 {
        c.camera.position()
    }

    /// Run `axes` (shaped by `settings`) through the smoothing and the
    /// camera for `seconds` at `fps`.
    fn run(c: &mut OrbitCamera, settings: &Settings, axes: [f32; 6], seconds: f32, fps: f32, pivot: Vec3) {
        let mut smoothed = Smoothed::default();
        let dt = 1.0 / fps;
        let frames = (seconds * fps).round() as usize;
        for _ in 0..frames {
            smoothed.follow(settings.shape(axes), dt);
            drive(c, smoothed.mean, dt, 1.5, pivot, settings.sensitivity);
        }
    }

    /// Inside the dead zone nothing moves; full deflection is still 1;
    /// inverting turns an axis round; dominant keeps only the strongest.
    #[test]
    fn the_dead_zone_inverts_and_the_dominant_axis_shape_the_input() {
        let mut settings = Settings { dead_zone: 0.1, ..Settings::default() };
        assert_eq!(settings.shape([0.09, -0.05, 0.0, 0.1, 0.0, 0.0]), [0.0; 6], "inside the dead zone");
        let full = settings.shape([1.0, -1.0, 0.0, 0.0, 0.0, 0.0]);
        assert!((full[0] - 1.0).abs() < 1e-6 && (full[1] + 1.0).abs() < 1e-6, "{full:?}");
        let half = settings.shape([0.55, 0.0, 0.0, 0.0, 0.0, 0.0])[0];
        assert!(half > 0.2 && half < 0.5, "a gentle curve below full deflection: {half}");
        settings.invert[2] = true;
        assert!(settings.shape([0.0, 0.0, 0.8, 0.0, 0.0, 0.0])[2] < 0.0, "zoom inverted");
        settings.dominant = true;
        let one = settings.shape([0.5, 0.0, 0.0, 0.0, -0.7, 0.3]);
        assert_eq!(one.iter().filter(|v| **v != 0.0).count(), 1);
        assert!(one[4] < 0.0, "the spin, pushed furthest, is the one kept: {one:?}");
    }

    /// The cap moves the model: right moves it right (the eye goes left),
    /// pulling it toward you brings it closer, lifting raises it.
    #[test]
    fn the_cap_moves_the_model() {
        let settings = Settings::default();
        let start = camera();
        let right = start.camera.camera_to_world.transform_vector(Vec3::X);

        let mut c = start.clone();
        run(&mut c, &settings, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.5, 60.0, start.target);
        assert!((eye(&c) - eye(&start)).dot(right) < -0.1, "pushed right, the camera went left");
        assert!((c.distance - start.distance).abs() < 1e-4, "a pan changes no distance");

        let mut c = start.clone();
        run(&mut c, &settings, [0.0, 1.0, 0.0, 0.0, 0.0, 0.0], 0.5, 60.0, start.target);
        assert!(eye(&c).y < eye(&start).y, "lifted, the model rose: the camera went down");

        let mut c = start.clone();
        run(&mut c, &settings, [0.0, 0.0, 1.0, 0.0, 0.0, 0.0], 0.5, 60.0, start.target);
        assert!(c.distance < start.distance * 0.7, "pulled toward you, the model came closer: {} -> {}", start.distance, c.distance);
        let mut c = start.clone();
        run(&mut c, &settings, [0.0, 0.0, -1.0, 0.0, 0.0, 0.0], 0.5, 60.0, start.target);
        assert!(c.distance > start.distance * 1.3, "pushed away, it went further");
    }

    /// Twisting clockwise from above turns the model clockwise (the camera
    /// goes round anticlockwise); tilting forward tips the model away (the
    /// camera drops); the roll moves nothing.
    #[test]
    fn twisting_and_tilting_turn_the_model() {
        let settings = Settings::default();
        let start = camera();
        let mut c = start.clone();
        run(&mut c, &settings, [0.0, 0.0, 0.0, 0.0, -1.0, 0.0], 0.5, 60.0, start.target);
        assert!(c.yaw > start.yaw + 0.3, "{} -> {}", start.yaw, c.yaw);
        assert!((c.distance - start.distance).abs() < 1e-4);

        let mut c = start.clone();
        run(&mut c, &settings, [0.0, 0.0, 0.0, -1.0, 0.0, 0.0], 0.3, 60.0, start.target);
        assert!(c.pitch < start.pitch - 0.1, "tilted forward: {} -> {}", start.pitch, c.pitch);

        let mut c = start.clone();
        run(&mut c, &settings, [0.0, 0.0, 0.0, 0.0, 0.0, 1.0], 0.5, 60.0, start.target);
        assert_eq!(c.camera.camera_to_world, start.camera.camera_to_world, "the roll moved the view");
    }

    /// The same push for the same time moves the view alike at 30 frames a
    /// second and at 144, smoothing and all.
    #[test]
    fn the_view_moves_alike_at_any_frame_rate() {
        let settings = Settings::default();
        let axes = [0.4, -0.3, 0.6, 0.5, -0.8, 0.0];
        let pivot = vec3(0.2, 0.1, -0.1);
        let (mut slow, mut fast) = (camera(), camera());
        run(&mut slow, &settings, axes, 1.0, 30.0, pivot);
        run(&mut fast, &settings, axes, 1.0, 144.0, pivot);
        let apart = (eye(&slow) - eye(&fast)).length();
        let moved = (eye(&fast) - eye(&camera())).length();
        assert!(apart < moved * 0.02, "30 and 144 fps ended {apart} apart after a move of {moved}");
        assert!((slow.distance / fast.distance - 1.0).abs() < 0.01);
    }

    /// Panning goes by the distance: from twice as far, the same push moves
    /// the camera twice as far, so the model crosses the view alike.
    #[test]
    fn panning_scales_with_the_distance_to_the_model() {
        let settings = Settings::default();
        let near = camera();
        let mut far = camera();
        far.zoom(-(2.0f32.ln()) / ZOOM_PER_NOTCH);
        assert!((far.distance / near.distance - 2.0).abs() < 1e-3);
        let (mut a, mut b) = (near.clone(), far.clone());
        run(&mut a, &settings, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.5, 60.0, near.target);
        run(&mut b, &settings, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.5, 60.0, far.target);
        let ratio = (b.target - far.target).length() / (a.target - near.target).length();
        assert!((ratio - 2.0).abs() < 0.02, "{ratio}");
    }

    /// Turning about a pivot off the view's centre swings the camera round
    /// it without a jump: the pivot stays the same distance from the eye,
    /// and a turn there and back returns the view.
    #[test]
    fn turning_about_a_pivot_keeps_it_in_place() {
        let start = camera();
        let pivot = vec3(0.4, 0.2, 0.3);
        let before = (pivot - eye(&start)).length();
        let mut c = start.clone();
        turn_about(&mut c, pivot, 0.4, 0.2);
        assert!(((pivot - eye(&c)).length() - before).abs() < 1e-4, "the pivot moved relative to the eye");
        // The pivot's place in the view (its direction from the eye, in the
        // camera's frame) is unchanged by the turn.
        let seen = |c: &OrbitCamera| c.camera.camera_to_world.inverse().map(|m| m.transform_vector(pivot - eye(c)).normalize());
        let (a, b) = (seen(&start).unwrap(), seen(&c).unwrap());
        assert!((a - b).length() < 1e-3, "the pivot moved across the view: {a:?} -> {b:?}");
        turn_about(&mut c, pivot, 0.0, -0.2);
        turn_about(&mut c, pivot, -0.4, 0.0);
        assert!((eye(&c) - eye(&start)).length() < 1e-3);
        assert!((c.target - start.target).length() < 1e-3);
    }

    /// Faster with the sensitivity up, and not at all inside the dead zone.
    #[test]
    fn sensitivity_scales_every_move() {
        let start = camera();
        let mut slow = start.clone();
        let mut quick = start.clone();
        run(&mut slow, &Settings::default(), [0.0, 0.0, 0.0, 0.0, -1.0, 0.0], 0.2, 60.0, start.target);
        run(&mut quick, &Settings { sensitivity: 2.0, ..Settings::default() }, [0.0, 0.0, 0.0, 0.0, -1.0, 0.0], 0.2, 60.0, start.target);
        assert!(((quick.yaw - start.yaw) / (slow.yaw - start.yaw) - 2.0).abs() < 0.01);
        let mut still = start.clone();
        run(&mut still, &Settings::default(), [0.03, -0.02, 0.04, 0.0, 0.05, 0.0], 0.5, 60.0, start.target);
        assert_eq!(still.camera.camera_to_world, start.camera.camera_to_world, "inside the dead zone the view moved");
    }

    #[test]
    fn button_actions_are_kept_by_name() {
        for action in ButtonAction::ALL {
            assert_eq!(ButtonAction::from_key(action.key()), Some(action));
        }
    }
}
