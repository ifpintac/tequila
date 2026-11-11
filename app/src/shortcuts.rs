//! The keyboard shortcuts (`docs/04` §6.2, §6.3): every command a key can
//! give, the keys it has by default, and the person's own.
//!
//! The keys were written into the code that read them, one `if` a key.
//! They are a table now ([`ACTIONS`]): the main window asks which actions
//! this frame's key presses give ([`Shortcuts::pressed`]) and carries them
//! out, and Preferences › Keyboard changes which keys those are. The
//! Shortcuts window and the menus' tooltips show the keys from the same
//! table, so they say what the keys are now.
//!
//! The graph editor's and the UV editor's own keys are theirs, and are not
//! here.

use eframe::egui;

/// A key with the modifiers held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: egui::Key,
    /// Ctrl, or Cmd on a Mac.
    pub command: bool,
    /// A Mac's own Control key, which is not its Cmd: Cmd+H is the
    /// system's, so hiding parts is Control+H there. Never set elsewhere,
    /// where Control is `command`.
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    pub const fn plain(key: egui::Key) -> Chord {
        Chord { key, command: false, control: false, shift: false, alt: false }
    }

    /// The chord of a key pressed with `modifiers`.
    pub fn pressed(key: egui::Key, modifiers: &egui::Modifiers) -> Chord {
        Chord::pressed_on(key, modifiers, cfg!(target_os = "macos"))
    }

    fn pressed_on(key: egui::Key, modifiers: &egui::Modifiers, mac: bool) -> Chord {
        // Elsewhere than a Mac, Ctrl is the command key and nothing else.
        Chord { key, command: modifiers.command || (!mac && modifiers.ctrl), control: mac && modifiers.ctrl, shift: modifiers.shift, alt: modifiers.alt }
    }

    /// Written out for a person, in the words the Shortcuts window uses:
    /// `Ctrl + Shift + A`. `theme::keys` and `theme::key_caps` turn Ctrl
    /// and Alt into a Mac's Cmd and Option.
    pub fn shown(&self) -> String {
        self.written(key_symbol(self.key))
    }

    /// Written for the settings file: the same, the key by its name, so a
    /// comma is `Comma` and a list of chords can be told apart.
    pub fn to_text(&self) -> String {
        self.written(self.key.name())
    }

    fn written(&self, key: &str) -> String {
        let mut out = String::new();
        for (held, name) in [(self.command, "Ctrl"), (self.control, "Control"), (self.alt, "Alt"), (self.shift, "Shift")] {
            if held {
                out.push_str(name);
                out.push_str(" + ");
            }
        }
        out.push_str(key);
        out
    }

    /// Read back, from the file or a script: modifiers in any order and
    /// any case, the key by its name or its symbol.
    pub fn parse(text: &str) -> Result<Chord, String> {
        Chord::parse_on(text, cfg!(target_os = "macos"))
    }

    fn parse_on(text: &str, mac: bool) -> Result<Chord, String> {
        let text = text.trim();
        // The key is what follows the last `+` that joins: `Ctrl + +`
        // would be the plus key, which is written `Plus`.
        let mut pieces: Vec<&str> = text.split('+').map(str::trim).collect();
        let key = pieces.pop().filter(|k| !k.is_empty()).ok_or_else(|| format!("`{text}` names no key"))?;
        let mut chord = Chord::plain(key_named(key).ok_or_else(|| format!("`{key}` is not a key"))?);
        for piece in pieces {
            match piece.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "command" => chord.command = true,
                // A Mac's Control; elsewhere the one Ctrl there is.
                "control" if mac => chord.control = true,
                "control" => chord.command = true,
                "shift" => chord.shift = true,
                "alt" | "option" | "opt" => chord.alt = true,
                other => return Err(format!("`{other}` in `{text}` is not Ctrl, Control, Shift or Alt")),
            }
        }
        Ok(chord)
    }
}

/// A key as the Shortcuts window writes it: punctuation as itself.
fn key_symbol(key: egui::Key) -> &'static str {
    use egui::Key;
    match key {
        Key::Comma => ",",
        Key::Period => ".",
        Key::Slash => "/",
        Key::Backtick => "`",
        Key::Minus => "-",
        Key::Equals => "=",
        Key::Semicolon => ";",
        Key::Quote => "'",
        Key::Backslash => "\\",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Escape => "Esc",
        other => other.name(),
    }
}

fn key_named(name: &str) -> Option<egui::Key> {
    match name.to_ascii_lowercase().as_str() {
        "esc" => Some(egui::Key::Escape),
        "del" => Some(egui::Key::Delete),
        "plus" => Some(egui::Key::Plus),
        _ => egui::Key::from_name(name).or_else(|| {
            // Whatever the case: `f5`, `tab`, `a`.
            egui::Key::ALL.iter().copied().find(|key| key.name().eq_ignore_ascii_case(name))
        }),
    }
}

/// Everything a key can ask of the main window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Undo,
    Redo,
    SelectAll,
    DeselectAll,
    Duplicate,
    Group,
    Delete,
    HideSelection,
    UnhideAll,
    Isolate,
    GizmoAll,
    GizmoMove,
    GizmoRotate,
    GizmoScale,
    FrameSelection,
    FrameAll,
    ViewFront,
    ViewBack,
    ViewLeft,
    ViewRight,
    ViewTop,
    ViewBottom,
    ViewIsometric,
    NextShading,
    RenderedAndBack,
    Statistics,
    SelectionOutline,
    Gizmo,
    Guides,
    TurnGuides,
    FullViewport,
    PauseViewport,
    Screenshot,
    Present,
    BackgroundEnvironment,
    BackgroundColour,
    BackgroundBackplate,
    Inspector,
    Library,
    Snapshots,
    Console,
    RenderWindow,
    Preferences,
    NewProject,
    OpenProject,
    Save,
    SaveAs,
    Import,
}

/// The headings the actions are listed under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Edit,
    Gizmo,
    Camera,
    View,
    Background,
    Panels,
    Files,
}

impl Group {
    pub const ALL: [Group; 7] = [Group::Edit, Group::Gizmo, Group::Camera, Group::View, Group::Background, Group::Panels, Group::Files];

    pub fn label(self) -> &'static str {
        match self {
            Group::Edit => "Edit",
            Group::Gizmo => "Gizmo",
            Group::Camera => "Camera",
            Group::View => "View",
            Group::Background => "Background",
            Group::Panels => "Panels and windows",
            Group::Files => "Files",
        }
    }
}

/// An action as the table has it.
pub struct Info {
    pub action: Action,
    /// Its name in the settings file and a script: `edit.undo`.
    pub key: &'static str,
    pub group: Group,
    pub label: &'static str,
    /// Its keys out of the box, as [`Chord::parse`] reads them.
    defaults: &'static [&'static str],
    /// On a Mac instead, where they differ.
    mac_defaults: Option<&'static [&'static str]>,
    /// Means the same over the graph editor and the UV editor, whose own
    /// keys otherwise have the keyboard there.
    pub everywhere: bool,
}

const fn info(action: Action, key: &'static str, group: Group, label: &'static str, defaults: &'static [&'static str]) -> Info {
    Info { action, key, group, label, defaults, mac_defaults: None, everywhere: false }
}

/// Every action, in the order it is listed.
pub static ACTIONS: &[Info] = &[
    Info { everywhere: true, ..info(Action::Undo, "edit.undo", Group::Edit, "Undo", &["Ctrl + Z"]) },
    Info { everywhere: true, ..info(Action::Redo, "edit.redo", Group::Edit, "Redo", &["Ctrl + Y", "Ctrl + Shift + Z"]) },
    info(Action::SelectAll, "edit.select_all", Group::Edit, "Select all", &["Ctrl + A"]),
    info(Action::DeselectAll, "edit.deselect_all", Group::Edit, "Deselect all", &["Esc", "Ctrl + Shift + A"]),
    info(Action::Duplicate, "edit.duplicate", Group::Edit, "Duplicate", &["Ctrl + D"]),
    info(Action::Group, "edit.group", Group::Edit, "Group", &["Ctrl + G"]),
    info(Action::Delete, "edit.delete", Group::Edit, "Delete", &["Delete", "Backspace"]),
    Info { mac_defaults: Some(&["Control + H"]), ..info(Action::HideSelection, "edit.hide", Group::Edit, "Hide the selection", &["Ctrl + H"]) },
    info(Action::UnhideAll, "edit.unhide_all", Group::Edit, "Show everything", &["Alt + H"]),
    info(Action::Isolate, "edit.isolate", Group::Edit, "Isolate the selection", &["/"]),
    info(Action::GizmoAll, "gizmo.all", Group::Gizmo, "Move, turn and scale at once", &["Q"]),
    info(Action::GizmoMove, "gizmo.move", Group::Gizmo, "Move", &["W"]),
    info(Action::GizmoRotate, "gizmo.rotate", Group::Gizmo, "Rotate", &["E"]),
    info(Action::GizmoScale, "gizmo.scale", Group::Gizmo, "Scale", &["R"]),
    info(Action::FrameSelection, "camera.frame_selection", Group::Camera, "Frame the selection", &["F"]),
    info(Action::FrameAll, "camera.frame_all", Group::Camera, "Frame everything", &["Shift + F"]),
    info(Action::ViewFront, "camera.front", Group::Camera, "Front view", &["Ctrl + Alt + 1"]),
    info(Action::ViewBack, "camera.back", Group::Camera, "Back view", &["Ctrl + Alt + 2"]),
    info(Action::ViewLeft, "camera.left", Group::Camera, "Left view", &["Ctrl + Alt + 3"]),
    info(Action::ViewRight, "camera.right", Group::Camera, "Right view", &["Ctrl + Alt + 4"]),
    info(Action::ViewTop, "camera.top", Group::Camera, "Top view", &["Ctrl + Alt + 5"]),
    info(Action::ViewBottom, "camera.bottom", Group::Camera, "Bottom view", &["Ctrl + Alt + 6"]),
    info(Action::ViewIsometric, "camera.isometric", Group::Camera, "Isometric view", &["Ctrl + Alt + 7"]),
    info(Action::NextShading, "view.next_shading", Group::View, "Step through the shadings", &["Z"]),
    info(Action::RenderedAndBack, "view.rendered", Group::View, "Rendered, and back", &["D"]),
    info(Action::Statistics, "view.statistics", Group::View, "Render statistics", &["H"]),
    info(Action::SelectionOutline, "view.selection_outline", Group::View, "Selection outline", &["Shift + H"]),
    info(Action::Gizmo, "view.gizmo", Group::View, "Gizmo shown or hidden", &["Shift + G"]),
    info(Action::Guides, "view.guides", Group::View, "Composition guides", &["O"]),
    info(Action::TurnGuides, "view.turn_guides", Group::View, "Turn the composition guides", &["Shift + O"]),
    info(Action::FullViewport, "view.full_viewport", Group::View, "Full viewport", &["Tab"]),
    info(Action::PauseViewport, "view.pause", Group::View, "Pause the viewport", &["Shift + P"]),
    info(Action::Screenshot, "view.screenshot", Group::View, "Screenshot of the viewport", &["P"]),
    info(Action::Present, "view.present", Group::View, "Presentation mode", &["F5"]),
    info(Action::BackgroundEnvironment, "background.environment", Group::Background, "Background: the environment", &["E"]),
    info(Action::BackgroundColour, "background.colour", Group::Background, "Background: a colour", &["C"]),
    info(Action::BackgroundBackplate, "background.backplate", Group::Background, "Background: a backplate", &["B"]),
    info(Action::Inspector, "panels.inspector", Group::Panels, "Inspector", &["Space"]),
    info(Action::Library, "panels.library", Group::Panels, "Material library", &["M"]),
    info(Action::Snapshots, "panels.snapshots", Group::Panels, "Snapshots", &["U"]),
    info(Action::Console, "panels.console", Group::Panels, "Console", &["`"]),
    info(Action::RenderWindow, "panels.render", Group::Panels, "Render window", &["Ctrl + R"]),
    info(Action::Preferences, "panels.preferences", Group::Panels, "Preferences", &["Ctrl + ,"]),
    info(Action::NewProject, "files.new", Group::Files, "New project", &["Ctrl + N"]),
    info(Action::OpenProject, "files.open", Group::Files, "Open a project", &["Ctrl + O"]),
    Info { everywhere: true, ..info(Action::Save, "files.save", Group::Files, "Save", &["Ctrl + S"]) },
    info(Action::SaveAs, "files.save_as", Group::Files, "Save as", &["Ctrl + Shift + S"]),
    info(Action::Import, "files.import", Group::Files, "Import a model", &["Ctrl + I"]),
];

/// The most keys one action has.
pub const SLOTS: usize = 2;

impl Info {
    fn default_chords(&self) -> Vec<Chord> {
        self.default_chords_on(cfg!(target_os = "macos"))
    }

    fn default_chords_on(&self, mac: bool) -> Vec<Chord> {
        let texts = self.mac_defaults.filter(|_| mac).unwrap_or(self.defaults);
        texts.iter().filter_map(|text| Chord::parse_on(text, mac).ok()).collect()
    }
}

impl Action {
    pub fn info(self) -> &'static Info {
        ACTIONS.iter().find(|info| info.action == self).expect("every action is in the table")
    }

    fn index(self) -> usize {
        ACTIONS.iter().position(|info| info.action == self).expect("every action is in the table")
    }

    pub fn named(key: &str) -> Option<Action> {
        ACTIONS.iter().find(|info| info.key.eq_ignore_ascii_case(key.trim())).map(|info| info.action)
    }
}

/// Whether two actions may have one key between them: Rotate and the
/// environment background do, as §6.2 has it - `E` turns the selection when
/// there is one and sets the background when there is not.
pub fn share(a: Action, b: Action) -> bool {
    matches!((a, b), (Action::GizmoRotate, Action::BackgroundEnvironment) | (Action::BackgroundEnvironment, Action::GizmoRotate))
}

/// Which keys give which actions: the defaults, and the person's changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Shortcuts {
    /// Each action's keys, as [`ACTIONS`] orders them.
    bound: Vec<Vec<Chord>>,
}

impl Default for Shortcuts {
    fn default() -> Shortcuts {
        Shortcuts { bound: ACTIONS.iter().map(Info::default_chords).collect() }
    }
}

impl Shortcuts {
    /// An action's keys.
    pub fn of(&self, action: Action) -> &[Chord] {
        &self.bound[action.index()]
    }

    /// An action's keys for the Shortcuts window and a tooltip: `Ctrl + Z
    /// / Ctrl + Y`, or nothing when it has none.
    pub fn shown(&self, action: Action) -> String {
        self.of(action).iter().map(Chord::shown).collect::<Vec<_>>().join(" / ")
    }

    /// The action that has `chord`, other than `but` and those that may
    /// share with it.
    pub fn owner(&self, chord: Chord, but: Action) -> Option<Action> {
        ACTIONS.iter().zip(&self.bound).find(|(info, chords)| info.action != but && !share(info.action, but) && chords.contains(&chord)).map(|(info, _)| info.action)
    }

    /// Whether an action's keys are other than its defaults.
    pub fn changed(&self, action: Action) -> bool {
        self.of(action) != action.info().default_chords().as_slice()
    }

    /// Give `action` these keys, at most [`SLOTS`], and take them from any
    /// other action that had one.
    pub fn bind(&mut self, action: Action, chords: &[Chord]) {
        let mut own: Vec<Chord> = Vec::new();
        for chord in chords.iter().take(SLOTS) {
            if !own.contains(chord) {
                own.push(*chord);
            }
        }
        for (info, bound) in ACTIONS.iter().zip(self.bound.iter_mut()) {
            if info.action != action && !share(info.action, action) {
                bound.retain(|chord| !own.contains(chord));
            }
        }
        self.bound[action.index()] = own;
    }

    /// Put `chord` in one of `action`'s places, taking it from whichever
    /// action had it; `None` empties the place.
    pub fn set(&mut self, action: Action, slot: usize, chord: Option<Chord>) {
        let mut chords = self.of(action).to_vec();
        match chord {
            Some(chord) if slot < chords.len() => chords[slot] = chord,
            Some(chord) => chords.push(chord),
            None if slot < chords.len() => {
                chords.remove(slot);
            }
            None => {}
        }
        self.bind(action, &chords);
    }

    /// An action's keys back to its defaults, which are taken from any
    /// other action that had been given one.
    pub fn reset(&mut self, action: Action) {
        self.bind(action, &action.info().default_chords());
    }

    /// The actions this frame's key presses give, each press with the
    /// modifiers it was pressed with. A key held down repeats, as it
    /// always has: holding Ctrl+Z walks back through the history.
    pub fn pressed(&self, ctx: &egui::Context) -> Vec<Action> {
        ctx.input(|input| {
            let mut out = Vec::new();
            for event in &input.events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = event {
                    let chord = Chord::pressed(*key, modifiers);
                    for (info, chords) in ACTIONS.iter().zip(&self.bound) {
                        if chords.contains(&chord) && !out.contains(&info.action) {
                            out.push(info.action);
                        }
                    }
                }
            }
            out
        })
    }

    /// The changed actions as lines of the settings file: the action's
    /// name and its keys, or nothing after the `=` for none.
    pub fn changed_lines(&self) -> Vec<(String, String)> {
        ACTIONS.iter().filter(|info| self.changed(info.action)).map(|info| (info.key.to_string(), self.text_of(info.action))).collect()
    }

    /// Every action's line, for the exported file.
    pub fn all_lines(&self) -> Vec<(String, String)> {
        ACTIONS.iter().map(|info| (info.key.to_string(), self.text_of(info.action))).collect()
    }

    fn text_of(&self, action: Action) -> String {
        self.of(action).iter().map(Chord::to_text).collect::<Vec<_>>().join(", ")
    }

    /// Take a line of the file: an action's name and its keys.
    pub fn read_line(&mut self, name: &str, keys: &str) -> Result<(), String> {
        let action = Action::named(name).ok_or_else(|| format!("there is no shortcut `{}`", name.trim()))?;
        let chords = parse_list(keys)?;
        self.bind(action, &chords);
        Ok(())
    }
}

/// Keys as a file's line lists them: by name, with commas between.
pub fn parse_list(keys: &str) -> Result<Vec<Chord>, String> {
    keys.split(',').map(str::trim).filter(|k| !k.is_empty()).map(Chord::parse).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Key;

    /// The table is whole: every action once, named for its group, with
    /// defaults that read, no two actions on one key but the pair that
    /// share by design, on a Mac and elsewhere.
    #[test]
    fn every_action_is_in_the_table_with_keys_of_its_own() {
        for (i, info) in ACTIONS.iter().enumerate() {
            assert!(ACTIONS[..i].iter().all(|other| other.action != info.action && other.key != info.key), "{} is there twice", info.key);
            assert!(!info.label.is_empty() && info.key.contains('.'));
            assert_eq!(Action::named(info.key), Some(info.action));
            for mac in [false, true] {
                let chords = info.default_chords_on(mac);
                assert_eq!(chords.len(), info.mac_defaults.filter(|_| mac).unwrap_or(info.defaults).len(), "{}'s defaults do not all read", info.key);
                assert!(chords.len() <= SLOTS);
                for other in &ACTIONS[..i] {
                    if !share(other.action, info.action) {
                        assert!(other.default_chords_on(mac).iter().all(|c| !chords.contains(c)), "{} and {} have one key", other.key, info.key);
                    }
                }
            }
        }
        assert_eq!(Shortcuts::default().shown(Action::Redo), "Ctrl + Y / Ctrl + Shift + Z");
        assert_eq!(Shortcuts::default().shown(Action::Console), "`");
        assert_eq!(Shortcuts::default().shown(Action::Preferences), "Ctrl + ,");
    }

    /// A chord reads back from what it writes, whatever the case and the
    /// order of its modifiers; a Mac's Control is its own key there and
    /// Ctrl elsewhere.
    #[test]
    fn a_chord_reads_back() {
        for info in ACTIONS {
            for chord in info.default_chords() {
                assert_eq!(Chord::parse(&chord.to_text()), Ok(chord));
                assert_eq!(Chord::parse(&chord.shown()), Ok(chord), "{}", chord.shown());
            }
        }
        let wanted = Chord { key: Key::A, command: true, control: false, shift: true, alt: false };
        assert_eq!(Chord::parse("shift+CTRL + a"), Ok(wanted));
        assert_eq!(Chord::parse("Cmd+Shift+A"), Ok(wanted));
        assert_eq!(Chord::parse("f5"), Ok(Chord::plain(Key::F5)));
        assert_eq!(Chord::parse("Ctrl + Comma").map(|c| c.shown()), Ok("Ctrl + ,".to_string()));
        assert_eq!(Chord::parse("Ctrl + Comma").map(|c| c.to_text()), Ok("Ctrl + Comma".to_string()));
        assert!(Chord::parse("Hyper + A").unwrap_err().contains("not Ctrl"));
        assert!(Chord::parse("Ctrl + Banana").unwrap_err().contains("not a key"));
        assert!(Chord::parse("Ctrl +").is_err());
        let mac = Chord::parse_on("Control + H", true).unwrap();
        assert!(mac.control && !mac.command);
        let other = Chord::parse_on("Control + H", false).unwrap();
        assert!(other.command && !other.control);
        // As pressed: Ctrl is the command key on Windows, and a Mac's
        // Control is not its Cmd.
        assert_eq!(Chord::pressed_on(Key::H, &egui::Modifiers::CTRL, false), other);
        assert_eq!(Chord::pressed_on(Key::H, &egui::Modifiers::CTRL, true), mac);
        assert_eq!(Chord::pressed_on(Key::H, &egui::Modifiers::MAC_CMD.plus(egui::Modifiers::COMMAND), true), Chord { command: true, ..Chord::plain(Key::H) });
    }

    /// A key given to an action is taken from the one that had it, but
    /// for the pair that share; a reset gives the defaults back.
    #[test]
    fn a_key_belongs_to_one_action() {
        let mut keys = Shortcuts::default();
        let g = Chord::parse("Ctrl + G").unwrap();
        assert_eq!(keys.owner(g, Action::Duplicate), Some(Action::Group));
        assert_eq!(keys.owner(g, Action::Group), None, "an action's own key is not a conflict");
        keys.set(Action::Duplicate, 1, Some(g));
        assert_eq!(keys.shown(Action::Duplicate), "Ctrl + D / Ctrl + G");
        assert!(keys.of(Action::Group).is_empty(), "Group kept a key it gave up");
        assert!(keys.changed(Action::Group) && keys.changed(Action::Duplicate) && !keys.changed(Action::Undo));

        // E is Rotate's and the environment's, and stays both.
        let e = Chord::plain(Key::E);
        assert_eq!(keys.owner(e, Action::GizmoRotate), None);
        keys.set(Action::GizmoRotate, 0, Some(e));
        assert_eq!(keys.of(Action::BackgroundEnvironment), &[e]);

        // A place emptied, and a third key that has no place.
        keys.set(Action::Duplicate, 0, None);
        assert_eq!(keys.shown(Action::Duplicate), "Ctrl + G");
        keys.bind(Action::Redo, &[Chord::plain(Key::F1), Chord::plain(Key::F2), Chord::plain(Key::F3), Chord::plain(Key::F1)]);
        assert_eq!(keys.shown(Action::Redo), "F1 / F2");

        keys.reset(Action::Group);
        assert_eq!(keys.shown(Action::Group), "Ctrl + G");
        assert!(keys.of(Action::Duplicate).is_empty(), "the reset did not take Group's key back");
    }

    /// Only what changed is kept, and it reads back; a line that names no
    /// action or no key is said.
    #[test]
    fn changed_shortcuts_are_kept_as_lines_and_read_back() {
        let mut keys = Shortcuts::default();
        assert!(keys.changed_lines().is_empty());
        keys.set(Action::Screenshot, 0, Some(Chord::parse("Ctrl + Shift + ,").unwrap()));
        keys.set(Action::Console, 0, None);
        let lines = keys.changed_lines();
        assert_eq!(lines, vec![("view.screenshot".to_string(), "Ctrl + Shift + Comma".to_string()), ("panels.console".to_string(), String::new())]);
        let mut read = Shortcuts::default();
        for (name, value) in &lines {
            read.read_line(name, value).unwrap();
        }
        assert_eq!(read, keys);
        let mut all = Shortcuts::default();
        for (name, value) in keys.all_lines() {
            all.read_line(&name, &value).unwrap();
        }
        assert_eq!(all, keys);
        assert!(read.read_line("view.nothing", "A").unwrap_err().contains("no shortcut"));
        assert!(read.read_line("view.screenshot", "Ctrl + Banana").is_err());
        assert_eq!(read, keys, "a bad line changed something");
    }

    /// A press gives its action, with the modifiers it was pressed with
    /// and no others: Ctrl+Shift+G is not Ctrl+G.
    #[test]
    fn a_press_gives_the_action_bound_to_exactly_that_chord() {
        let keys = Shortcuts::default();
        let ctx = egui::Context::default();
        let pressed = |key: Key, modifiers: egui::Modifiers| -> Vec<Action> {
            let mut out = Vec::new();
            let input = egui::RawInput { modifiers, events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() };
            let _ = ctx.run(input, |ctx| out = keys.pressed(ctx));
            out
        };
        let command = egui::Modifiers { ctrl: !cfg!(target_os = "macos"), mac_cmd: cfg!(target_os = "macos"), command: true, ..Default::default() };
        assert_eq!(pressed(Key::G, command), vec![Action::Group]);
        assert!(pressed(Key::G, egui::Modifiers { shift: true, ..command }).is_empty());
        assert_eq!(pressed(Key::G, egui::Modifiers::SHIFT), vec![Action::Gizmo]);
        assert_eq!(pressed(Key::E, egui::Modifiers::NONE), vec![Action::GizmoRotate, Action::BackgroundEnvironment]);
        assert_eq!(pressed(Key::Z, egui::Modifiers { shift: true, ..command }), vec![Action::Redo]);
        assert!(pressed(Key::F9, egui::Modifiers::NONE).is_empty());
    }
}
