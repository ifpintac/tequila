//! How the mouse moves the view (`docs/04` §6.1): which button, with which
//! keys held, orbits, pans and zooms.
//!
//! It was KeyShot's way and no other: the left button orbits, the middle
//! pans, Alt and the right button dollies. Someone coming from Blender,
//! Maya or a CAD package has other hands, so each of those is a preset
//! here, and Custom is any three drags of the person's own (Preferences ›
//! Navigation). The wheel zooms in every one of them.

use eframe::egui;

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

impl Button {
    pub const ALL: [Button; 3] = [Button::Left, Button::Middle, Button::Right];

    fn key(self) -> &'static str {
        match self {
            Button::Left => "left",
            Button::Middle => "middle",
            Button::Right => "right",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Button::Left => "Left",
            Button::Middle => "Middle",
            Button::Right => "Right",
        }
    }

    pub fn pointer(self) -> egui::PointerButton {
        match self {
            Button::Left => egui::PointerButton::Primary,
            Button::Middle => egui::PointerButton::Middle,
            Button::Right => egui::PointerButton::Secondary,
        }
    }
}

/// A drag: a button, with the keys held while it is down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drag {
    pub button: Button,
    /// Ctrl, or Cmd on a Mac.
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Drag {
    pub const fn new(button: Button) -> Drag {
        Drag { button, ctrl: false, shift: false, alt: false }
    }

    const fn with(button: Button, ctrl: bool, shift: bool, alt: bool) -> Drag {
        Drag { button, ctrl, shift, alt }
    }

    fn keys_held(&self) -> usize {
        usize::from(self.ctrl) + usize::from(self.shift) + usize::from(self.alt)
    }

    /// Whether this drag is being made: its button, with at least its keys.
    fn made_by(&self, button: Button, modifiers: &egui::Modifiers) -> bool {
        self.button == button && (!self.ctrl || modifiers.command) && (!self.shift || modifiers.shift) && (!self.alt || modifiers.alt)
    }

    /// Its name in the settings file and a script: `ctrl+shift+middle`.
    pub fn key(&self) -> String {
        let mut out = String::new();
        for (held, name) in [(self.ctrl, "ctrl+"), (self.alt, "alt+"), (self.shift, "shift+")] {
            if held {
                out.push_str(name);
            }
        }
        out.push_str(self.button.key());
        out
    }

    pub fn from_key(key: &str) -> Option<Drag> {
        let mut pieces: Vec<String> = key.split('+').map(|p| p.trim().to_ascii_lowercase()).collect();
        let button = pieces.pop()?;
        let mut drag = Drag::new(Button::ALL.into_iter().find(|b| b.key() == button)?);
        for piece in pieces {
            match piece.as_str() {
                "ctrl" | "cmd" => drag.ctrl = true,
                "shift" => drag.shift = true,
                "alt" | "option" => drag.alt = true,
                _ => return None,
            }
        }
        Some(drag)
    }

    /// For the Shortcuts window's key caps: `Alt + Right drag`.
    pub fn shown(&self) -> String {
        let mut out = String::new();
        for (held, name) in [(self.ctrl, "Ctrl + "), (self.alt, "Alt + "), (self.shift, "Shift + ")] {
            if held {
                out.push_str(name);
            }
        }
        out.push_str(self.button.label());
        out.push_str(" drag");
        out
    }
}

/// What a drag does to the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Orbit,
    Pan,
    /// Closer and further, by dragging up and down.
    Zoom,
}

/// The three drags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scheme {
    pub orbit: Drag,
    pub pan: Drag,
    pub zoom: Drag,
}

impl Scheme {
    /// What dragging `button` with `modifiers` held does. Of the drags that
    /// fit, the one asking for the most keys: with Blender's, Shift and
    /// the middle button pans though the middle button alone orbits. Keys
    /// a drag does not ask for do no harm, as they never have: Shift and
    /// a left drag still orbits in KeyShot's.
    pub fn move_for(&self, button: Button, modifiers: &egui::Modifiers) -> Option<Move> {
        [(Move::Orbit, self.orbit), (Move::Pan, self.pan), (Move::Zoom, self.zoom)]
            .into_iter()
            .filter(|(_, drag)| drag.made_by(button, modifiers))
            .max_by_key(|(what, drag)| (drag.keys_held(), std::cmp::Reverse(*what as u8)))
            .map(|(what, _)| what)
    }

    pub fn drag(&self, what: Move) -> Drag {
        match what {
            Move::Orbit => self.orbit,
            Move::Pan => self.pan,
            Move::Zoom => self.zoom,
        }
    }

    /// Give `what` the drag `to`. A move that had it takes the one given
    /// up, so no two share a drag and a swap is one choice.
    pub fn set(&mut self, what: Move, to: Drag) {
        let from = self.drag(what);
        for other in [Move::Orbit, Move::Pan, Move::Zoom] {
            if other != what && self.drag(other) == to {
                *self.slot(other) = from;
            }
        }
        *self.slot(what) = to;
    }

    fn slot(&mut self, what: Move) -> &mut Drag {
        match what {
            Move::Orbit => &mut self.orbit,
            Move::Pan => &mut self.pan,
            Move::Zoom => &mut self.zoom,
        }
    }
}

/// Whose way of moving the view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preset {
    /// Tequila's own, which is KeyShot's.
    #[default]
    KeyShot,
    Blender,
    Maya,
    Cinema4D,
    Max,
    SolidWorks,
    Rhino,
    Fusion,
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 9] =
        [Preset::KeyShot, Preset::Blender, Preset::Maya, Preset::Cinema4D, Preset::Max, Preset::SolidWorks, Preset::Rhino, Preset::Fusion, Preset::Custom];

    pub fn key(self) -> &'static str {
        match self {
            Preset::KeyShot => "keyshot",
            Preset::Blender => "blender",
            Preset::Maya => "maya",
            Preset::Cinema4D => "cinema4d",
            Preset::Max => "3dsmax",
            Preset::SolidWorks => "solidworks",
            Preset::Rhino => "rhino",
            Preset::Fusion => "fusion360",
            Preset::Custom => "custom",
        }
    }

    pub fn from_key(key: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.key().eq_ignore_ascii_case(key.trim()))
    }

    /// Its drags; `None` for Custom, whose are the person's.
    pub fn scheme(self) -> Option<Scheme> {
        use Button::{Left, Middle, Right};
        let d = Drag::with;
        Some(match self {
            // Left orbits, middle pans, Alt and right dollies (§6.1).
            Preset::KeyShot => Scheme { orbit: d(Left, false, false, false), pan: d(Middle, false, false, false), zoom: d(Right, false, false, true) },
            // The middle button for everything: alone, with Shift, with Ctrl.
            Preset::Blender => Scheme { orbit: d(Middle, false, false, false), pan: d(Middle, false, true, false), zoom: d(Middle, true, false, false) },
            // Alt and each button in turn; Cinema 4D's are the same.
            Preset::Maya | Preset::Cinema4D => Scheme { orbit: d(Left, false, false, true), pan: d(Middle, false, false, true), zoom: d(Right, false, false, true) },
            // The middle button pans; Alt with it orbits, Ctrl and Alt zoom.
            Preset::Max => Scheme { orbit: d(Middle, false, false, true), pan: d(Middle, false, false, false), zoom: d(Middle, true, false, true) },
            // The middle button orbits; Ctrl with it pans, Shift zooms.
            Preset::SolidWorks => Scheme { orbit: d(Middle, false, false, false), pan: d(Middle, true, false, false), zoom: d(Middle, false, true, false) },
            // The right button, as in a perspective view there.
            Preset::Rhino => Scheme { orbit: d(Right, false, false, false), pan: d(Right, false, true, false), zoom: d(Right, true, false, false) },
            // The middle button pans; Shift with it orbits.
            Preset::Fusion => Scheme { orbit: d(Middle, false, true, false), pan: d(Middle, false, false, false), zoom: d(Middle, true, true, false) },
            Preset::Custom => return None,
        })
    }
}

/// Every drag there is, for a setting's choices: each a key and its words,
/// in this computer's names for the keys.
macro_rules! drag_options {
    ($ctrl:literal, $alt:literal; $(($key:literal, $name:literal)),*) => {
        &[$(
            ($key, concat!($name, " drag")),
            (concat!("shift+", $key), concat!("Shift + ", $name, " drag")),
            (concat!("ctrl+", $key), concat!($ctrl, " + ", $name, " drag")),
            (concat!("alt+", $key), concat!($alt, " + ", $name, " drag")),
            (concat!("ctrl+shift+", $key), concat!($ctrl, " + Shift + ", $name, " drag")),
            (concat!("alt+shift+", $key), concat!($alt, " + Shift + ", $name, " drag")),
            (concat!("ctrl+alt+", $key), concat!($ctrl, " + ", $alt, " + ", $name, " drag")),
            (concat!("ctrl+alt+shift+", $key), concat!($ctrl, " + ", $alt, " + Shift + ", $name, " drag"))
        ),*]
    };
}

#[cfg(target_os = "macos")]
pub const DRAGS: &[(&str, &str)] = drag_options!("Cmd", "Option"; ("left", "Left"), ("middle", "Middle"), ("right", "Right"));
#[cfg(not(target_os = "macos"))]
pub const DRAGS: &[(&str, &str)] = drag_options!("Ctrl", "Alt"; ("left", "Left"), ("middle", "Middle"), ("right", "Right"));

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: egui::Modifiers = egui::Modifiers::NONE;
    fn held(ctrl: bool, shift: bool, alt: bool) -> egui::Modifiers {
        egui::Modifiers { command: ctrl, ctrl, shift, alt, ..Default::default() }
    }

    /// Every preset's three drags differ, read back from their names, and
    /// are among the choices offered.
    #[test]
    fn every_preset_has_three_drags_of_its_own() {
        for preset in Preset::ALL {
            assert_eq!(Preset::from_key(preset.key()), Some(preset));
            let Some(scheme) = preset.scheme() else {
                assert_eq!(preset, Preset::Custom);
                continue;
            };
            assert!(scheme.orbit != scheme.pan && scheme.pan != scheme.zoom && scheme.orbit != scheme.zoom, "{preset:?}");
            for drag in [scheme.orbit, scheme.pan, scheme.zoom] {
                assert_eq!(Drag::from_key(&drag.key()), Some(drag));
                assert!(DRAGS.iter().any(|(key, _)| *key == drag.key()), "{} is not a choice", drag.key());
            }
        }
        assert_eq!(DRAGS.len(), 24);
        for (key, label) in DRAGS {
            let drag = Drag::from_key(key).unwrap_or_else(|| panic!("{key} does not read"));
            assert_eq!(drag.key(), *key);
            assert!(label.ends_with(&format!("{} drag", drag.button.label())), "{label}");
        }
        assert_eq!(Drag::from_key("Cmd + Shift + Middle").map(|d| d.shown()), Some("Ctrl + Shift + Middle drag".to_string()));
        assert_eq!(Drag::from_key("left+left"), None);
    }

    /// Tequila's own is what it always was: left orbits whatever is held,
    /// middle pans, Alt and right dollies, and a plain right drag nothing.
    #[test]
    fn the_default_moves_the_view_as_it_always_has() {
        let scheme = Preset::default().scheme().unwrap();
        assert_eq!(scheme.move_for(Button::Left, &NONE), Some(Move::Orbit));
        assert_eq!(scheme.move_for(Button::Left, &held(false, true, false)), Some(Move::Orbit));
        assert_eq!(scheme.move_for(Button::Middle, &NONE), Some(Move::Pan));
        assert_eq!(scheme.move_for(Button::Right, &held(false, false, true)), Some(Move::Zoom));
        assert_eq!(scheme.move_for(Button::Right, &NONE), None);
    }

    /// With several drags on one button, the one whose keys are held wins.
    #[test]
    fn the_drag_with_the_most_keys_held_wins() {
        let blender = Preset::Blender.scheme().unwrap();
        assert_eq!(blender.move_for(Button::Middle, &NONE), Some(Move::Orbit));
        assert_eq!(blender.move_for(Button::Middle, &held(false, true, false)), Some(Move::Pan));
        assert_eq!(blender.move_for(Button::Middle, &held(true, false, false)), Some(Move::Zoom));
        assert_eq!(blender.move_for(Button::Left, &NONE), None, "the left button is for selecting there");
        let maya = Preset::Maya.scheme().unwrap();
        assert_eq!(maya.move_for(Button::Left, &NONE), None);
        assert_eq!(maya.move_for(Button::Left, &held(false, false, true)), Some(Move::Orbit));
        assert_eq!(maya.move_for(Button::Middle, &held(false, false, true)), Some(Move::Pan));
        assert_eq!(maya.move_for(Button::Right, &held(false, false, true)), Some(Move::Zoom));
        let max = Preset::Max.scheme().unwrap();
        assert_eq!(max.move_for(Button::Middle, &held(false, false, true)), Some(Move::Orbit));
        assert_eq!(max.move_for(Button::Middle, &held(true, false, true)), Some(Move::Zoom));
        let fusion = Preset::Fusion.scheme().unwrap();
        assert_eq!(fusion.move_for(Button::Middle, &held(true, true, false)), Some(Move::Zoom));
        assert_eq!(fusion.move_for(Button::Middle, &held(false, true, false)), Some(Move::Orbit));
    }

    /// A drag given to one move is swapped with the move that had it.
    #[test]
    fn a_drag_given_away_is_swapped() {
        let mut scheme = Preset::KeyShot.scheme().unwrap();
        let (left, middle) = (scheme.orbit, scheme.pan);
        scheme.set(Move::Orbit, middle);
        assert_eq!((scheme.orbit, scheme.pan), (middle, left));
        let free = Drag::from_key("shift+right").unwrap();
        scheme.set(Move::Zoom, free);
        assert_eq!(scheme.zoom, free);
        assert_eq!((scheme.orbit, scheme.pan), (middle, left), "a free drag moved another");
    }
}
