//! The editor's keys, active only while it is under the pointer or has the
//! keyboard, so they never reach the viewport's: there `W`, `E` and `R` are
//! the gizmo's, Delete deletes parts and Ctrl+R opens the Render window.
//! Here the same keys mean the same kind of thing for the layout - tools,
//! deleting labels, the rulers - and undo, redo and save mean what they
//! mean everywhere.

use super::selection::Mode;
use super::tools::Tool;
use tequila_core::math::{vec2, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Tool(Tool),
    Mode(Mode),
    /// A nudge one step this way, or ten with Shift.
    Nudge(Vec2, bool),
    Delete,
    Duplicate,
    SelectAll,
    Escape,
    FrameSelection,
    FrameAll,
    Zoom(f32),
    Fit,
    Grid,
    Guides,
    Snapping,
    Rulers,
    Help,
    Cut,
    Stitch,
    /// Pinned, or with false unpinned.
    Pin(bool),
    Pack,
    /// Quarter turns, clockwise as the editor shows them.
    Quarter(i32),
    /// Flipped across (left for right), or down.
    Flip(bool),
    /// One of the shape tools.
    Shape(super::shape_tools::Tool),
}

/// Every key, by group, for the help popover: the keys as key caps, and
/// what they do. In three columns, as `COLUMNS` splits them.
pub const LIST: &[(&str, &[(&str, &str)])] = &[
    (
        "Tools",
        &[("V", "Select"), ("W", "Move"), ("E", "Rotate"), ("R", "Scale"), ("M", "Measure")],
    ),
    (
        "What a click picks",
        &[("1", "Labels"), ("2", "Islands"), ("3", "Edges"), ("4", "Vertices")],
    ),
    (
        "Selection",
        &[
            ("Ctrl + A", "Select everything the mode picks"),
            ("Esc", "Select nothing, clear the measure"),
            ("Delete", "Delete the selected labels or guide"),
            ("Ctrl + D", "Duplicate the selected labels"),
            ("Arrows", "Nudge the selection"),
            ("Shift + Arrows", "Nudge ten times as far"),
        ],
    ),
    (
        "While dragging",
        &[
            ("Ctrl", "No snapping for this drag"),
            ("Shift", "Keep it straight: a move across or down, a turn in 15° steps, a measure at 0, 45 or 90°; a scale's width and height apart"),
            ("Alt", "Scale about the middle"),
        ],
    ),
    (
        "Island tools",
        &[
            ("C", "Cut along the selected edges"),
            ("S", "Stitch the selected seams, or the seams between the selected islands"),
            ("P", "Pin the selected vertices"),
            ("Shift + P", "Unpin them"),
            ("Ctrl + P", "Pack the selected islands, or all"),
            ("]", "Turn a quarter clockwise"),
            ("[", "Turn a quarter anticlockwise"),
            ("Shift + H", "Flip horizontal"),
            ("Shift + V", "Flip vertical"),
        ],
    ),
    (
        "Shape tools",
        &[
            ("Alt + U", "Unwrap again, as its options say"),
            ("Alt + L", "Straight line"),
            ("Alt + G", "Grid"),
            ("Alt + B", "Rectangle"),
            ("Alt + P", "Project from the faces' direction"),
            ("Alt + R", "Relax"),
            ("Alt + O", "Circle"),
            ("Alt + W", "Weld nearby"),
            ("Alt + M", "Match seam sides"),
            ("Alt + Y", "Symmetrize"),
        ],
    ),
    (
        "View",
        &[
            ("F", "Frame the selection"),
            ("Home", "Frame everything"),
            ("Ctrl + 0", "Fit"),
            ("Ctrl + 1", "Zoom to 100%"),
            ("Ctrl + 2", "Zoom to 200%"),
            ("Wheel", "Zoom about the pointer"),
            ("Middle drag", "Pan"),
            ("Right drag", "Pan"),
            ("Right click", "What can be done to the selection"),
        ],
    ),
    (
        "Show",
        &[
            ("Ctrl + '", "The grid"),
            ("Ctrl + ;", "Guides"),
            ("Ctrl + Shift + ;", "Snapping"),
            ("Ctrl + R", "Rulers"),
            ("?", "These shortcuts"),
        ],
    ),
    (
        "Everywhere",
        &[("Ctrl + Z", "Undo"), ("Ctrl + Shift + Z", "Redo"), ("Ctrl + S", "Save")],
    ),
];

/// Where the popover's columns begin in `LIST`.
pub const COLUMNS: [usize; 3] = [0, 3, 6];

/// The actions pressed this frame, each with the modifiers it was pressed
/// with.
pub fn read(ctx: &egui::Context) -> Vec<Action> {
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } => action(*key, *modifiers),
                _ => None,
            })
            .collect()
    })
}

/// What one key does here.
pub fn action(key: egui::Key, modifiers: egui::Modifiers) -> Option<Action> {
    use egui::Key;
    let command = modifiers.command;
    let plain = !command && !modifiers.alt;
    let only_shift = modifiers.shift && !command && !modifiers.alt;
    let only_alt = modifiers.alt && !command && !modifiers.shift;
    if only_alt {
        use super::shape_tools::Tool;
        let tool = match key {
            Key::U => Tool::Unwrap,
            Key::L => Tool::Straight,
            Key::G => Tool::Grid,
            Key::B => Tool::Rectangle,
            Key::P => Tool::Project,
            Key::R => Tool::Relax,
            Key::O => Tool::Circle,
            Key::W => Tool::Weld,
            Key::M => Tool::MatchSeams,
            Key::Y => Tool::Symmetrize,
            _ => return None,
        };
        return Some(Action::Shape(tool));
    }
    Some(match key {
        Key::V if !modifiers.any() => Action::Tool(Tool::Select),
        Key::W if !modifiers.any() => Action::Tool(Tool::Move),
        Key::E if !modifiers.any() => Action::Tool(Tool::Rotate),
        Key::R if !modifiers.any() => Action::Tool(Tool::Scale),
        Key::M if !modifiers.any() => Action::Tool(Tool::Measure),
        Key::Num1 if !modifiers.any() => Action::Mode(Mode::Labels),
        Key::Num2 if !modifiers.any() => Action::Mode(Mode::Islands),
        Key::Num3 if !modifiers.any() => Action::Mode(Mode::Edges),
        Key::Num4 if !modifiers.any() => Action::Mode(Mode::Vertices),
        Key::C if !modifiers.any() => Action::Cut,
        Key::S if !modifiers.any() => Action::Stitch,
        Key::P if !modifiers.any() => Action::Pin(true),
        Key::P if only_shift => Action::Pin(false),
        Key::P if command && !modifiers.shift => Action::Pack,
        Key::CloseBracket if !modifiers.any() => Action::Quarter(1),
        Key::OpenBracket if !modifiers.any() => Action::Quarter(-1),
        Key::H if only_shift => Action::Flip(true),
        Key::V if only_shift => Action::Flip(false),
        Key::ArrowLeft if plain => Action::Nudge(vec2(-1.0, 0.0), modifiers.shift),
        Key::ArrowRight if plain => Action::Nudge(vec2(1.0, 0.0), modifiers.shift),
        Key::ArrowUp if plain => Action::Nudge(vec2(0.0, -1.0), modifiers.shift),
        Key::ArrowDown if plain => Action::Nudge(vec2(0.0, 1.0), modifiers.shift),
        Key::Delete | Key::Backspace if !command => Action::Delete,
        Key::D if command && !modifiers.shift => Action::Duplicate,
        Key::A if command && !modifiers.shift => Action::SelectAll,
        Key::Escape => Action::Escape,
        Key::F if !modifiers.any() => Action::FrameSelection,
        Key::Home => Action::FrameAll,
        Key::Num0 if command => Action::Fit,
        Key::Num1 if command => Action::Zoom(1.0),
        Key::Num2 if command => Action::Zoom(2.0),
        Key::Quote if command => Action::Grid,
        Key::Semicolon if command && modifiers.shift => Action::Snapping,
        // Shift with ; is a colon on most keyboards.
        Key::Colon if command => Action::Snapping,
        Key::Semicolon if command => Action::Guides,
        Key::R if command && !modifiers.shift => Action::Rulers,
        Key::Questionmark => Action::Help,
        Key::Slash if modifiers.shift && !command => Action::Help,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_do_what_the_list_says() {
        let none = egui::Modifiers::NONE;
        let ctrl = egui::Modifiers::COMMAND;
        let shift_ctrl = egui::Modifiers { shift: true, ..egui::Modifiers::COMMAND };
        assert_eq!(action(egui::Key::V, none), Some(Action::Tool(Tool::Select)));
        assert_eq!(action(egui::Key::M, none), Some(Action::Tool(Tool::Measure)));
        assert_eq!(action(egui::Key::R, none), Some(Action::Tool(Tool::Scale)));
        assert_eq!(action(egui::Key::R, ctrl), Some(Action::Rulers));
        assert_eq!(action(egui::Key::Quote, ctrl), Some(Action::Grid));
        assert_eq!(action(egui::Key::Semicolon, ctrl), Some(Action::Guides));
        assert_eq!(action(egui::Key::Semicolon, shift_ctrl), Some(Action::Snapping));
        assert_eq!(action(egui::Key::ArrowLeft, egui::Modifiers::SHIFT), Some(Action::Nudge(vec2(-1.0, 0.0), true)));
        assert_eq!(action(egui::Key::D, ctrl), Some(Action::Duplicate));
        assert_eq!(action(egui::Key::Z, ctrl), None, "undo is everywhere's, not the editor's");
        // Every key the list names for a tool is the tool's, and for a mode
        // the mode's.
        for info in super::super::tools::TOOLS {
            let key = egui::Key::from_name(info.key).unwrap();
            assert_eq!(action(key, none), Some(Action::Tool(info.tool)), "{}", info.name);
            assert!(LIST[0].1.iter().any(|(k, what)| *k == info.key && *what == info.name));
        }
        for mode in Mode::ALL {
            let key = egui::Key::from_name(mode.key_name()).unwrap();
            assert_eq!(action(key, none), Some(Action::Mode(mode)));
            assert!(LIST[1].1.iter().any(|(k, what)| *k == mode.key_name() && *what == mode.name()));
        }
        // Ctrl with a number is still the zoom.
        assert_eq!(action(egui::Key::Num1, ctrl), Some(Action::Zoom(1.0)));
        assert_eq!(action(egui::Key::P, none), Some(Action::Pin(true)));
        assert_eq!(action(egui::Key::P, egui::Modifiers::SHIFT), Some(Action::Pin(false)));
        assert_eq!(action(egui::Key::P, ctrl), Some(Action::Pack));
        assert_eq!(action(egui::Key::V, egui::Modifiers::SHIFT), Some(Action::Flip(false)), "Shift + V flips, V alone selects");
        assert_eq!(action(egui::Key::CloseBracket, none), Some(Action::Quarter(1)));
        // The shape tools on Alt and a letter, as the list says, and each
        // tool's own keys are those.
        let alt = egui::Modifiers::ALT;
        let shape = LIST.iter().find(|(title, _)| *title == "Shape tools").unwrap().1;
        for (keys, _) in shape {
            let letter = keys.trim_start_matches("Alt + ");
            let Some(Action::Shape(tool)) = action(egui::Key::from_name(letter).unwrap(), alt) else { panic!("{keys} is no shape tool") };
            assert_eq!(tool.keys(), *keys, "{}", tool.name());
        }
        assert_eq!(action(egui::Key::R, alt), Some(Action::Shape(super::super::shape_tools::Tool::Relax)), "Alt + R relaxes, R alone scales");
        // Each group heads a column, and every row names one key.
        assert_eq!(COLUMNS[0], 0);
        assert!(COLUMNS.windows(2).all(|w| w[0] < w[1]) && *COLUMNS.last().unwrap() < LIST.len());
        let shifts = LIST.iter().find(|(title, _)| *title == "While dragging").unwrap().1.iter().filter(|(k, _)| *k == "Shift").count();
        assert_eq!(shifts, 1, "one Shift row while dragging, saying all it does");
    }
}
