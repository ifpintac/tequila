//! The 2D cursor: a point of the layout the person puts down - Shift and a
//! right click, landing on whatever a drag would snap to - for Align to line
//! things up against and for turns and scales to go about. Each part has its
//! own, kept for the session; a script reads it from the editor's state.
//!
//! Shift with a right click rather than a right click alone: a right click
//! opens the menu and a right drag pans, and neither takes Shift, so the
//! cursor is put down without taking either away.

use super::layout::Layout;
use super::view::Xform;
use super::{Part, UvEditor};
use std::collections::HashMap;
use tequila_core::math::{vec2, Vec2};
use tequila_materials::decal::Decal;
use tequila_scene::NodeId;

/// The cursors, one for each part, and what they are for.
#[derive(Clone, Debug, Default)]
pub struct Cursors {
    places: HashMap<NodeId, Vec2>,
    /// Turns and scales of the layout's own go about the cursor rather
    /// than the selection's middle or the Transform panel's chosen point.
    pub pivot: bool,
    /// The point they go about this frame, where they go about the cursor.
    pub about: Option<Vec2>,
}

impl Cursors {
    pub fn of(&self, node: NodeId) -> Option<Vec2> {
        self.places.get(&node).copied()
    }

    pub fn set(&mut self, node: NodeId, at: Vec2) {
        self.places.insert(node, at);
    }

    pub fn clear(&mut self, node: NodeId) {
        self.places.remove(&node);
    }

    pub fn forget(&mut self) {
        self.places.clear();
        self.about = None;
    }
}

impl UvEditor {
    /// The part's cursor, if it has one.
    pub(super) fn cursor_of(&self, part: &Part) -> Option<Vec2> {
        self.cursors.of(part.node)
    }

    /// What turns and scales of the layout's own go about: the cursor where
    /// asked and there is one, else `otherwise`.
    pub(super) fn pivot_or(&self, part: &Part, otherwise: Vec2) -> Vec2 {
        if self.cursors.pivot { self.cursor_of(part).unwrap_or(otherwise) } else { otherwise }
    }

    /// The cursor put down at a point of the screen, snapped as a drag
    /// would be there.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_cursor(&mut self, at: egui::Pos2, xform: &Xform, part: &Part, layout: &Layout, labels: &[Decal], canvas: egui::Rect) {
        let uv = xform.to_layout(at);
        self.targets = self.gather_targets(part, layout, labels, &[], &[], None, xform, canvas);
        let snap = self.snap_point(uv, xform);
        let at = uv + snap.delta;
        self.targets = super::snapping::Targets::new();
        self.put_cursor(part, at, snap.note());
    }

    /// The cursor at `at`, and a word saying where.
    pub(super) fn put_cursor(&mut self, part: &Part, at: Vec2, snapped: Option<&str>) {
        self.cursors.set(part.node, at);
        if !self.prefs.show_cursor {
            self.prefs.show_cursor = true;
        }
        let place = format!("2D cursor at {}, {}", self.scale.format(at.x), self.scale.format(at.y));
        self.say(&match snapped {
            Some(note) => format!("{place}, on the {note}"),
            None => place,
        }, false);
    }

    /// The cursor on the middle of what is selected: the layout's own, or
    /// the labels.
    pub(super) fn cursor_to_selection(&mut self, part: &Part, layout: &Layout, labels: &[Decal]) {
        let middle = if self.selection.is_layout() {
            self.layout_frame(layout).map(|f| f.centre)
        } else {
            self.selection_frame(labels, part).map(|f| f.centre)
        };
        match middle {
            Some(at) => self.put_cursor(part, at, None),
            None => self.say("Could not move the cursor: select something to put it on", true),
        }
    }

    pub(super) fn cursor_to_centre(&mut self, part: &Part) {
        self.put_cursor(part, vec2(0.5, 0.5), None);
    }
}

/// The cursor drawn: a ring in red and white, so it reads over any artwork,
/// with a cross through it.
pub fn draw(painter: &egui::Painter, xform: &Xform, at: Vec2) {
    let centre = xform.to_screen(at);
    let red = crate::theme::p().danger;
    let white = egui::Color32::WHITE;
    let radius = 11.0;
    painter.circle_stroke(centre, radius, egui::Stroke::new(4.0_f32, white));
    // Red and white quarters, as a dashed ring.
    for k in 0..8 {
        if k % 2 == 0 {
            let (a0, a1) = (k as f32 * std::f32::consts::TAU / 8.0, (k + 1) as f32 * std::f32::consts::TAU / 8.0);
            let points: Vec<egui::Pos2> = (0..=6).map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / 6.0;
                centre + egui::vec2(a.cos(), a.sin()) * radius
            }).collect();
            painter.add(egui::Shape::line(points, egui::Stroke::new(2.5_f32, red)));
        }
    }
    let dark = egui::Color32::from_black_alpha(200);
    for (a, b) in [(egui::vec2(-radius - 8.0, 0.0), egui::vec2(-radius + 4.0, 0.0)), (egui::vec2(radius - 4.0, 0.0), egui::vec2(radius + 8.0, 0.0)), (egui::vec2(0.0, -radius - 8.0), egui::vec2(0.0, -radius + 4.0)), (egui::vec2(0.0, radius - 4.0), egui::vec2(0.0, radius + 8.0))] {
        painter.line_segment([centre + a, centre + b], egui::Stroke::new(1.5_f32, dark));
    }
    painter.circle_filled(centre, 1.5, dark);
}
