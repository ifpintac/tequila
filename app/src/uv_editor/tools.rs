//! The editor's tools, and what a drag with one is doing while it lasts.
//!
//! A tool decides what a press on the canvas begins: Select picks, moves
//! and draws a box; Move, Rotate and Scale do that to the selection from
//! anywhere; Measure draws a dimension. Handles on the selection work in all
//! of them but Measure. A drag in progress is a [`Gesture`], which the
//! canvas updates as the pointer moves, snapping it through
//! `super::snapping`, and which ends as an edit when the button is let go.
//!
//! The layout engine's tools - cut and stitch, pin, straighten - join
//! `Tool` and [`ENGINE_TOOLS`] when its editing is wired in: each begins a
//! gesture of its own, and ends it as an engine command.

use super::selection::Item;
use crate::icons::Icon;
use tequila_core::math::Vec2;
use tequila_texture::projection::Mapping;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Move,
    Rotate,
    Scale,
    Measure,
}

/// A tool as the toolbar and the shortcuts list show it.
pub struct ToolInfo {
    pub tool: Tool,
    pub icon: Icon,
    pub name: &'static str,
    pub detail: &'static str,
    pub key: &'static str,
}

pub const TOOLS: [ToolInfo; 5] = [
    ToolInfo { tool: Tool::Select, icon: Icon::Pointer, name: "Select", detail: "Click a label to select it, Shift-click to add one, drag on empty space to select with a box. Drag a label or its handles to move, scale or turn it.", key: "V" },
    ToolInfo { tool: Tool::Move, icon: Icon::Move, name: "Move", detail: "Drag anywhere to move the selection. Shift keeps to across or down.", key: "W" },
    ToolInfo { tool: Tool::Rotate, icon: Icon::Turn, name: "Rotate", detail: "Drag anywhere to turn the selection about its middle. Shift turns by 15°.", key: "E" },
    ToolInfo { tool: Tool::Scale, icon: Icon::Scale, name: "Scale", detail: "Drag anywhere to scale the selection about its middle, keeping its shape. Shift scales width and height apart.", key: "R" },
    ToolInfo { tool: Tool::Measure, icon: Icon::Ruler, name: "Measure", detail: "Drag to measure a distance and an angle on the part. Shift keeps to 0, 45 or 90°. Esc clears it.", key: "M" },
];

/// The layout engine's tools, in the toolbar's island group: none until its
/// editing is wired in.
pub const ENGINE_TOOLS: &[ToolInfo] = &[];

impl Tool {
    /// Its name on the scripting channel.
    pub fn key(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Move => "move",
            Tool::Rotate => "rotate",
            Tool::Scale => "scale",
            Tool::Measure => "measure",
        }
    }

    pub fn from_key(key: &str) -> Option<Tool> {
        TOOLS.iter().map(|t| t.tool).find(|t| t.key() == key.to_ascii_lowercase())
    }
}

/// Which handle of the selection's frame, or what a tool does from anywhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    /// The whole of it, moved.
    Move,
    /// A corner (`across`, `down` each -1 or 1) or the middle of an edge
    /// (one of them 0), dragged: scaled with the opposite side held, or
    /// about the middle with Alt.
    Handle { across: i8, down: i8 },
    /// Turned about its middle: by the stalk, or the Rotate tool.
    Turn,
    /// Scaled about its middle: the Scale tool.
    Scale,
}

/// The frame of what is being transformed, as the drag began: one label's
/// own, turned as it is, or the box square to the layout round several.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub centre: Vec2,
    pub size: Vec2,
    pub angle: f32,
}

impl Frame {
    /// Its own axes on the layout: across and down, turned.
    pub fn axes(&self) -> (Vec2, Vec2) {
        let (sin, cos) = self.angle.sin_cos();
        (tequila_core::math::vec2(cos, sin), tequila_core::math::vec2(-sin, cos))
    }

    /// The point a handle sits at: -1, 0 or 1 across and down from the
    /// middle.
    pub fn handle(&self, across: f32, down: f32) -> Vec2 {
        let (x, y) = self.axes();
        self.centre + x * (across * self.size.x * 0.5) + y * (down * self.size.y * 0.5)
    }

    /// Its four corners, from its top left round.
    pub fn corners(&self) -> [Vec2; 4] {
        [self.handle(-1.0, -1.0), self.handle(1.0, -1.0), self.handle(1.0, 1.0), self.handle(-1.0, 1.0)]
    }

    /// A point in its own frame: across and down from the middle.
    pub fn local(&self, p: Vec2) -> Vec2 {
        let (x, y) = self.axes();
        let d = p - self.centre;
        tequila_core::math::vec2(d.dot(x), d.dot(y))
    }
}

/// A drag in progress.
#[derive(Clone, Debug)]
pub enum Gesture {
    /// The selection moved, scaled or turned: each item as it began, and as
    /// it is now.
    Transform {
        grab: Grab,
        /// Where the pointer was pressed, on the layout.
        from: Vec2,
        frame: Frame,
        original: Vec<(Item, Mapping)>,
        current: Vec<(Item, Mapping)>,
    },
    /// A box drawn on empty space, selecting what it touches; `add` keeps
    /// what was selected.
    Box { from: Vec2, to: Vec2, add: bool },
    /// A guide dragged: a new one out of a ruler, or one already there, by
    /// its place in the part's list.
    Guide { index: Option<usize>, across: bool, at: f32, removing: bool },
    /// A dimension drawn by the Measure tool.
    Measure { from: Vec2, to: Vec2 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec2;

    #[test]
    fn a_frame_s_handles_turn_with_it() {
        let frame = Frame { centre: vec2(0.5, 0.5), size: vec2(0.4, 0.2), angle: std::f32::consts::FRAC_PI_2 };
        // Its right edge's middle, a quarter turn on, is below the middle.
        assert!((frame.handle(1.0, 0.0) - vec2(0.5, 0.7)).length() < 1e-5);
        assert!((frame.local(frame.handle(-1.0, 1.0)) - vec2(-0.2, 0.1)).length() < 1e-5);
        for tool in TOOLS.iter().map(|t| t.tool) {
            assert_eq!(Tool::from_key(tool.key()), Some(tool));
        }
    }
}
