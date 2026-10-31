//! The editor's keys, active only while it is under the pointer or has the
//! keyboard, so they never reach the viewport's: there `W`, `E` and `R` are
//! the gizmo's, Delete deletes parts and Ctrl+R opens the Render window.
//! Here the same keys mean the same kind of thing for the layout - tools,
//! deleting labels, the rulers - and undo, redo and save mean what they
//! mean everywhere.

use super::tools::Tool;
use tequila_core::math::{vec2, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Tool(Tool),
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
}

/// Every key, by group, for the help popover: the keys as key caps, and
/// what they do.
pub const LIST: &[(&str, &[(&str, &str)])] = &[
    (
        "Tools",
        &[("V", "Select"), ("W", "Move"), ("E", "Rotate"), ("R", "Scale"), ("M", "Measure")],
    ),
    (
        "Selection",
        &[
            ("Ctrl + A", "Select every label"),
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
            ("Shift", "Turn by 15°"),
            ("Shift", "Scale width and height apart"),
            ("Shift", "Move only across or down; measure at 0, 45 or 90°"),
            ("Alt", "Scale about the middle"),
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
    Some(match key {
        Key::V if !modifiers.any() => Action::Tool(Tool::Select),
        Key::W if !modifiers.any() => Action::Tool(Tool::Move),
        Key::E if !modifiers.any() => Action::Tool(Tool::Rotate),
        Key::R if !modifiers.any() => Action::Tool(Tool::Scale),
        Key::M if !modifiers.any() => Action::Tool(Tool::Measure),
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
        // Every key the list names for a tool is the tool's.
        for info in super::super::tools::TOOLS {
            let key = egui::Key::from_name(info.key).unwrap();
            assert_eq!(action(key, none), Some(Action::Tool(info.tool)), "{}", info.name);
            assert!(LIST[0].1.iter().any(|(k, what)| *k == info.key && *what == info.name));
        }
    }
}
