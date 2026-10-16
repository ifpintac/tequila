//! Orbit camera navigation: `tequila_scene::orbit`, shared with the
//! headless scripting server so a scripted `look` lands where it does here.

pub use tequila_scene::orbit::*;

/// A wheel notch's worth of scrolling, in points: what the viewport divides
/// a scroll by to get the notches [`OrbitCamera::zoom`] takes.
pub const POINTS_PER_NOTCH: f32 = 120.0;

/// How much one notch of [`OrbitCamera::zoom`] changes the distance, as the
/// exponent of the factor: a pinch by `f` is `ln f` of it.
const ZOOM_PER_NOTCH: f32 = 0.12;

/// What this frame's trackpad asks of the camera (`TEQUILA_SPEC.md` §1.7).
///
/// A MacBook has no middle button and no wheel, so it could orbit and do
/// nothing else. A two-finger swipe pans, as it moves a page, and a pinch
/// zooms. Shift + swipe orbits, for those who find a trackpad's click-drag
/// awkward. Cmd + swipe zooms and Alt + swipe changes the focal length, as
/// Ctrl and Alt + scroll do. A trackpad (or a Magic Mouse) is told from a
/// wheel by its unit: it reports points, and a wheel reports lines. Windows
/// reports lines for both, and gets none of this.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gestures {
    pub pan: egui::Vec2,
    pub orbit: egui::Vec2,
    /// In wheel notches, as [`OrbitCamera::zoom`] takes them: positive is in.
    pub zoom: f32,
    /// In wheel notches, as [`OrbitCamera::change_focal_length`] takes them.
    pub focal_length: f32,
    /// Whether a trackpad swiped this frame: the wheel's scroll, which
    /// counts the swipe too, is then not read as a wheel's.
    pub swiped: bool,
}

impl Gestures {
    pub fn read(events: &[egui::Event]) -> Gestures {
        let mut gestures = Gestures::default();
        for event in events {
            match event {
                egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, modifiers } => {
                    gestures.swiped = true;
                    if modifiers.alt {
                        gestures.focal_length += delta.y / POINTS_PER_NOTCH;
                    } else if modifiers.command || modifiers.ctrl {
                        gestures.zoom += delta.y / POINTS_PER_NOTCH;
                    } else if modifiers.shift {
                        gestures.orbit += *delta;
                    } else {
                        gestures.pan += *delta;
                    }
                }
                // A pinch by `factor`: the subject that much bigger.
                egui::Event::Zoom(factor) if factor.is_finite() && *factor > 0.0 => {
                    gestures.zoom += factor.ln() / ZOOM_PER_NOTCH;
                }
                _ => {}
            }
        }
        gestures
    }

    /// Whether they move the camera at all.
    pub fn moves(&self) -> bool {
        self.pan != egui::Vec2::ZERO || self.orbit != egui::Vec2::ZERO || self.zoom != 0.0 || self.focal_length != 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swipe(x: f32, y: f32, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(x, y), modifiers }
    }

    #[test]
    fn a_swipe_pans_and_shift_cmd_and_alt_change_what_it_does() {
        let none = egui::Modifiers::NONE;
        let pan = Gestures::read(&[swipe(3.0, -2.0, none), swipe(1.0, 1.0, none)]);
        assert_eq!((pan.pan, pan.orbit, pan.zoom, pan.swiped), (egui::vec2(4.0, -1.0), egui::Vec2::ZERO, 0.0, true));
        let orbit = Gestures::read(&[swipe(5.0, 2.0, egui::Modifiers::SHIFT)]);
        assert_eq!((orbit.orbit, orbit.pan), (egui::vec2(5.0, 2.0), egui::Vec2::ZERO));
        let zoom = Gestures::read(&[swipe(0.0, 60.0, egui::Modifiers::COMMAND)]);
        assert_eq!((zoom.zoom, zoom.pan), (0.5, egui::Vec2::ZERO));
        let focal = Gestures::read(&[swipe(0.0, -120.0, egui::Modifiers::ALT)]);
        assert_eq!((focal.focal_length, focal.zoom), (-1.0, 0.0));
    }

    #[test]
    fn a_wheel_is_left_to_the_wheel() {
        // Lines are a wheel's, Windows' touchpads' included: the viewport's
        // scroll handling keeps them, exactly as before.
        let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, 1.0), modifiers: egui::Modifiers::NONE };
        let page = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Page, delta: egui::vec2(0.0, 1.0), modifiers: egui::Modifiers::NONE };
        let read = Gestures::read(&[wheel, page, egui::Event::PointerGone]);
        assert_eq!(read, Gestures::default());
        assert!(!read.moves());
    }

    #[test]
    fn a_pinch_makes_the_subject_that_much_bigger() {
        let mut camera = OrbitCamera::framing(tequila_core::math::vec3(0.0, 1.0, 0.0), 1.0, 1.5);
        let before = camera.distance;
        let pinch = Gestures::read(&[egui::Event::Zoom(1.1), egui::Event::Zoom(1.25 / 1.1)]);
        assert!(pinch.moves() && !pinch.swiped);
        camera.zoom(pinch.zoom);
        assert!((before / camera.distance - 1.25).abs() < 1e-4, "{before} -> {}", camera.distance);
        // Apart again, back where it was; a nonsense factor is no pinch.
        camera.zoom(Gestures::read(&[egui::Event::Zoom(0.8)]).zoom);
        assert!((camera.distance - before).abs() < before * 1e-4);
        assert_eq!(Gestures::read(&[egui::Event::Zoom(0.0), egui::Event::Zoom(f32::NAN)]), Gestures::default());
    }
}
