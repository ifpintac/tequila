//! The canvas: the layout drawn - its square, the grid, the labels' and a
//! texture's artwork, the stretch heatmap, the islands' edges and seams,
//! pins, guides, the selection's handles, a measure - and the pointer on
//! it: picking what the mode picks, the drag a tool begins
//! (`super::tools::Gesture`), zoom and pan, and the context menu. The
//! rulers along its top and left are drawn here too, since a guide is
//! dragged from them onto it.

use super::editing;
use super::layout::{self, Layout};
use super::placement;
use super::rulers;
use super::selection::{Item, Mode};
use super::snapping::{self, Kind, Targets};
use super::tools::{Frame, Gesture, Grab, Tool};
use super::view::Xform;
use super::{Edits, Part, UvEditor};
use std::collections::HashSet;
use tequila_core::math::{vec2, Vec2};
use tequila_materials::decal::Decal;
use tequila_scene::guides::{GuideAxis, UvGuide};
use tequila_scene::uv::{EdgeKind, Measure, UvAffine};
use tequila_texture::projection::{Mapping, Projection};

/// How near, in points, the pointer has to be to a handle or a guide.
const GRAB: f32 = 7.0;
/// How far the turn handle stands out from the frame, in points.
const STALK: f32 = 24.0;
/// How far in the view has to be, in points per typical edge, for the
/// layout's vertices and edge middles to be snapped to.
const VERTICES_AT: f32 = 24.0;
/// How far in, the same way, for every vertex to be drawn in the Vertices
/// mode.
const DOTS_AT: f32 = 10.0;
/// Turns snap to this with Shift.
const TURN_STEP: f32 = std::f32::consts::PI / 12.0;

/// What is under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Handle(Grab),
    Item(Item),
    Guide(usize),
    Empty,
}

/// What the context menu was asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuAction {
    Duplicate,
    Delete,
    ToFront,
    ToBack,
    Flip(bool),
    /// Quarter turns, clockwise as shown.
    Quarter(i32),
    Fit,
    Centre,
    Along,
    Square,
    /// One of the island tools.
    Island(super::actions::Action),
    LockGuides,
    ClearGuides,
    FrameSelection,
    FrameAll,
    SelectAll,
}

/// Where each item of the selection is drawn this frame: the gesture's
/// copy while one is in progress, the labels as edited otherwise. None for
/// the layout's own items, which are drawn through the drag's map.
pub fn shown_mapping(gesture: Option<&Gesture>, item: Item, labels: &[Decal], part: &Part) -> Option<Mapping> {
    if let Some(Gesture::Transform { current, .. }) = gesture {
        if let Some((_, mapping)) = current.iter().find(|(i, _)| *i == item) {
            return Some(*mapping);
        }
    }
    match item {
        Item::Label(i) => labels.get(i).map(|d| d.mapping),
        Item::Texture(param) => part.textures.iter().find(|t| t.param == param).map(|t| t.mapping),
        Item::Island(_) | Item::Edge(_) | Item::Vertex(_) => None,
    }
}

/// Whether a label can be picked on the canvas: on the layout, shown and
/// not locked.
pub fn pickable(label: &Decal) -> bool {
    label.mapping.projection == Projection::Uv && label.enabled && !label.locked
}

/// The colours the layout's own are drawn in.
pub struct Colours {
    pub seam: egui::Color32,
    pub pin: egui::Color32,
    pub hover: egui::Color32,
    pub linked: egui::Color32,
    pub picked: egui::Color32,
}

pub fn colours() -> Colours {
    use crate::theme::{hue, Hue};
    Colours { seam: hue(Hue::Orange), pin: hue(Hue::Violet), hover: hue(Hue::Pink), linked: hue(Hue::Green), picked: hue(Hue::Blue) }
}

impl UvEditor {
    /// The frame of the selection as it is shown: one item's own, or the box
    /// round several labels.
    pub(super) fn selection_frame(&self, labels: &[Decal], part: &Part) -> Option<Frame> {
        let gesture = self.gesture.as_ref();
        let items = self.selection.items();
        if items.len() == 1 {
            let mapping = shown_mapping(gesture, items[0], labels, part)?;
            if mapping.projection != Projection::Uv {
                return None;
            }
            return Some(Frame { centre: placement::centre(&mapping), size: placement::size(&mapping), angle: placement::turn(&mapping) });
        }
        let mappings: Vec<Mapping> = items
            .iter()
            .filter_map(|item| shown_mapping(gesture, *item, labels, part))
            .filter(|m| m.projection == Projection::Uv)
            .collect();
        super::transform::frame_round(&mappings)
    }

    /// The frame the handles are on: the labels', or the box round the
    /// layout's own selection, as a drag or the panel shows it.
    fn frame_now(&self, labels: &[Decal], part: &Part, layout: &Layout) -> Option<Frame> {
        if !self.selection.is_layout() {
            return self.selection_frame(labels, part);
        }
        match &self.gesture {
            // Turned, the frame turns with it rather than growing round it.
            Some(Gesture::Layout { grab: Grab::Turn, frame, current, .. }) => Some(Frame { centre: current.apply(frame.centre), size: frame.size, angle: current.m[1][0].atan2(current.m[0][0]) }),
            Some(Gesture::Layout { frame, current, .. }) => {
                let corners = frame.corners().map(|c| current.apply(c));
                let low = corners.iter().fold(Vec2::splat(f32::INFINITY), |a, p| a.min(*p));
                let high = corners.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
                Some(Frame { centre: (low + high) * 0.5, size: high - low, angle: 0.0 })
            }
            _ => self.layout_frame(layout),
        }
    }

    /// The handles of the selection's frame on screen: its corners, the
    /// middles of its edges, and the turn handle's stalk.
    fn handles(&self, frame: &Frame, xform: &Xform, single_texture: bool) -> Vec<(Grab, egui::Pos2)> {
        let mut out = Vec::new();
        for (across, down) in [(-1, -1), (1, -1), (1, 1), (-1, 1)] {
            out.push((Grab::Handle { across, down }, xform.to_screen(frame.handle(across as f32, down as f32))));
        }
        if !single_texture {
            for (across, down) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                out.push((Grab::Handle { across, down }, xform.to_screen(frame.handle(across as f32, down as f32))));
            }
        }
        let top = xform.to_screen(frame.handle(0.0, -1.0));
        let middle = xform.to_screen(frame.centre);
        let outward = (top - middle).normalized();
        let outward = if outward.is_finite() { outward } else { egui::vec2(0.0, -1.0) };
        out.push((Grab::Turn, top + outward * STALK));
        out
    }

    /// What is under a point of the screen, of the labels.
    pub(super) fn pick(&self, at: egui::Pos2, xform: &Xform, labels: &[Decal], part: &Part) -> Hit {
        let uv = xform.to_layout(at);
        if self.tool != Tool::Measure {
            if let Some(frame) = self.selection_frame(labels, part) {
                let texture = self.selection.texture().is_some();
                for (grab, place) in self.handles(&frame, xform, texture) {
                    if (place - at).length() <= GRAB {
                        return Hit::Handle(grab);
                    }
                }
            }
        }
        for (index, label) in labels.iter().enumerate().rev() {
            if pickable(label) && layout::inside_quad(uv, &placement::corners(&label.mapping)) {
                return Hit::Item(Item::Label(index));
            }
        }
        // A texture repeats over everything, so its tile is picked only once
        // it is selected.
        if let Some(param) = self.selection.texture() {
            if let Some(mapping) = shown_mapping(self.gesture.as_ref(), Item::Texture(param), labels, part) {
                if layout::inside_quad(uv, &placement::corners(&mapping)) {
                    return Hit::Item(Item::Texture(param));
                }
            }
        }
        self.pick_guide(at, xform, part)
    }

    /// A guide near a point of the screen.
    fn pick_guide(&self, at: egui::Pos2, xform: &Xform, part: &Part) -> Hit {
        if self.prefs.guides && !self.prefs.guides_locked {
            for (index, guide) in part.guides.iter().enumerate().rev() {
                let place = match guide.axis {
                    GuideAxis::U => (xform.to_screen(vec2(guide.at, 0.0)).x - at.x).abs(),
                    GuideAxis::V => (xform.to_screen(vec2(0.0, guide.at)).y - at.y).abs(),
                };
                if place <= GRAB * 0.6 {
                    return Hit::Guide(index);
                }
            }
        }
        Hit::Empty
    }

    /// What is under a point of the screen, of what the mode picks.
    pub(super) fn pick_any(&self, at: egui::Pos2, xform: &Xform, labels: &[Decal], part: &Part, layout: &Layout) -> Hit {
        if self.mode == Mode::Labels {
            return self.pick(at, xform, labels, part);
        }
        if self.tool != Tool::Measure && self.selection.is_layout() {
            if let Some(frame) = self.frame_now(labels, part, layout) {
                for (grab, place) in self.handles(&frame, xform, false) {
                    if (place - at).length() <= GRAB {
                        return Hit::Handle(grab);
                    }
                }
            }
        }
        match editing::pick(layout, self.mode, xform.to_layout(at), xform.layout_length(GRAB)) {
            Some(item) => Hit::Item(item),
            None => self.pick_guide(at, xform, part),
        }
    }

    /// The grid's step on the layout, along each axis, and the finer one
    /// between its lines.
    pub(super) fn grid_steps(&self, density: Option<f32>) -> (Vec2, Vec2) {
        let major = match (self.prefs.spacing, density) {
            (super::prefs::Spacing::Length, Some(density)) => Vec2::splat((self.prefs.grid_mm / 1000.0 / density).max(1e-6)),
            _ => Vec2::splat(1.0 / self.prefs.divisions.max(1) as f32),
        };
        (major, major / self.prefs.subdivisions.max(1) as f32)
    }

    /// Everything a drag can snap to, but what it moves: labels, or the
    /// layout's islands and vertices.
    #[allow(clippy::too_many_arguments)]
    fn gather_targets(&self, part: &Part, layout: &Layout, labels: &[Decal], moving: &[Item], moving_vertices: &[u32], skip_guide: Option<usize>, xform: &Xform, canvas: egui::Rect) -> Targets {
        let mut targets = Targets::new();
        if self.prefs.grid {
            let (major, minor) = self.grid_steps(layout.density());
            // The finest lines drawn, which are what the eye aims for.
            let step = if minor.x * xform.scale >= 8.0 { minor } else { major };
            targets.grid = Some(snapping::Grid { step });
        }
        if self.prefs.guides {
            for (index, guide) in part.guides.iter().enumerate() {
                if Some(index) == skip_guide {
                    continue;
                }
                match guide.axis {
                    GuideAxis::U => targets.line_across(guide.at, f32::NEG_INFINITY, f32::INFINITY, Kind::Guide),
                    GuideAxis::V => targets.line_down(guide.at, f32::NEG_INFINITY, f32::INFINITY, Kind::Guide),
                }
            }
        }
        targets.rectangle(Vec2::ZERO, Vec2::splat(1.0), Kind::LayoutEdge, Kind::LayoutCentre);
        // What moves is no target for itself.
        let moving_islands: HashSet<u32> = moving_vertices.iter().map(|&v| layout.uv.uv_vertices[v as usize].island).collect();
        let whole = |island: u32| layout.uv.islands[island as usize].uv_vertices.iter().all(|v| moving_vertices.binary_search(v).is_ok());
        for (i, &(low, high)) in layout.boxes.iter().enumerate() {
            if moving_islands.contains(&(i as u32)) && whole(i as u32) {
                continue;
            }
            targets.rectangle(low, high, Kind::IslandEdge, Kind::IslandCentre);
        }
        if layout.typical_edge * xform.scale >= VERTICES_AT {
            let (low, high) = xform.visible(canvas);
            let seen = |p: &Vec2| p.x >= low.x && p.y >= low.y && p.x <= high.x && p.y <= high.y;
            let still: HashSet<(u32, u32)> = moving_vertices.iter().map(|&v| layout.uv.uv_vertices[v as usize].uv).map(|p| (p.x.to_bits(), p.y.to_bits())).collect();
            for p in layout.vertices.iter().filter(|p| seen(p) && !still.contains(&(p.x.to_bits(), p.y.to_bits()))).take(40_000) {
                targets.point(*p, Kind::Vertex);
            }
            if moving_vertices.is_empty() {
                for p in layout.midpoints.iter().filter(|p| seen(p)).take(40_000) {
                    targets.point(*p, Kind::Midpoint);
                }
            }
        }
        for (index, label) in labels.iter().enumerate() {
            if moving.contains(&Item::Label(index)) || label.mapping.projection != Projection::Uv || !label.enabled {
                continue;
            }
            let (low, high) = placement::bounds(&label.mapping);
            targets.rectangle(low, high, Kind::LabelEdge, Kind::LabelCentre);
        }
        targets
    }

    /// The selection's items and their mappings, to transform.
    fn originals(&self, labels: &[Decal], part: &Part) -> Vec<(Item, Mapping)> {
        self.selection
            .items()
            .iter()
            .filter(|item| match item {
                Item::Label(i) => labels.get(*i).is_some_and(|l| l.mapping.projection == Projection::Uv && !l.locked),
                Item::Texture(_) => true,
                _ => false,
            })
            .filter_map(|item| shown_mapping(None, *item, labels, part).map(|m| (*item, m)))
            .collect()
    }

    /// Begin a drag pressed at `at` on the screen.
    fn begin(&mut self, at: egui::Pos2, xform: &Xform, labels: &[Decal], part: &Part, layout: &Layout, canvas: egui::Rect) {
        let uv = xform.to_layout(at);
        let shift = self.modifiers.shift;
        if self.tool == Tool::Measure {
            self.targets = self.gather_targets(part, layout, labels, &[], &[], None, xform, canvas);
            let from = uv + self.snap_point(uv, xform).delta;
            self.gesture = Some(Gesture::Measure { from, to: from });
            return;
        }
        if self.mode.is_layout() {
            self.begin_layout(at, xform, labels, part, layout, canvas);
            return;
        }
        let transform = |editor: &mut UvEditor, grab: Grab| {
            let originals = editor.originals(labels, part);
            let frame = editor.selection_frame(labels, part);
            if let (false, Some(frame)) = (originals.is_empty(), frame) {
                let moving: Vec<Item> = originals.iter().map(|(i, _)| *i).collect();
                editor.targets = editor.gather_targets(part, layout, labels, &moving, &[], None, xform, canvas);
                editor.gesture = Some(Gesture::Transform { grab, from: uv, frame, original: originals.clone(), current: originals });
            }
        };
        let tool_grab = match self.tool {
            Tool::Rotate => Grab::Turn,
            Tool::Scale => Grab::Scale,
            _ => Grab::Move,
        };
        match self.pick(at, xform, labels, part) {
            Hit::Handle(grab) => transform(self, grab),
            Hit::Item(item) => {
                if !self.selection.contains(item) {
                    if shift {
                        self.selection.add(item);
                    } else {
                        self.selection.set(item);
                    }
                }
                self.selected_guide = None;
                transform(self, tool_grab);
            }
            Hit::Guide(index) => self.begin_guide_move(index, part, layout, labels, xform, canvas),
            Hit::Empty => {
                if self.tool != Tool::Select && !self.selection.is_empty() {
                    transform(self, tool_grab);
                }
                if self.gesture.is_none() {
                    self.gesture = Some(Gesture::Box { from: uv, to: uv, add: shift });
                }
            }
        }
    }

    /// A guide already there, dragged.
    fn begin_guide_move(&mut self, index: usize, part: &Part, layout: &Layout, labels: &[Decal], xform: &Xform, canvas: egui::Rect) {
        let guide = part.guides[index];
        self.selected_guide = Some(index);
        self.targets = self.gather_targets(part, layout, labels, &[], &[], Some(index), xform, canvas);
        self.gesture = Some(Gesture::Guide { index: Some(index), across: guide.axis == GuideAxis::U, at: guide.at, removing: false });
    }

    /// Begin a drag of the layout's own: a handle, an item picked, or a box.
    fn begin_layout(&mut self, at: egui::Pos2, xform: &Xform, labels: &[Decal], part: &Part, layout: &Layout, canvas: egui::Rect) {
        let uv = xform.to_layout(at);
        let shift = self.modifiers.shift;
        let start = |editor: &mut UvEditor, grab: Grab| {
            let moving = editing::moving(layout, &editor.selection);
            let Some(frame) = editing::frame_of(layout, &moving, &UvAffine::IDENTITY) else { return };
            editor.panel_preview = None;
            editor.targets = editor.gather_targets(part, layout, labels, &[], &moving, None, xform, canvas);
            editor.gesture = Some(Gesture::Layout { grab, from: uv, frame, current: UvAffine::IDENTITY, moving });
        };
        let tool_grab = match self.tool {
            Tool::Rotate => Grab::Turn,
            Tool::Scale => Grab::Scale,
            _ => Grab::Move,
        };
        match self.pick_any(at, xform, labels, part, layout) {
            Hit::Handle(grab) => start(self, grab),
            Hit::Item(item) => {
                if !self.selection.contains(item) {
                    if shift {
                        self.selection.add(item);
                    } else {
                        self.selection.set(item);
                    }
                }
                if let Item::Island(t) = item {
                    self.island = layout.uv.island_of.get(t as usize).map(|&i| i as usize);
                }
                self.selected_guide = None;
                start(self, tool_grab);
            }
            Hit::Guide(index) => self.begin_guide_move(index, part, layout, labels, xform, canvas),
            Hit::Empty => {
                if self.tool != Tool::Select && !self.selection.is_empty() {
                    start(self, tool_grab);
                }
                if self.gesture.is_none() {
                    self.gesture = Some(Gesture::Box { from: uv, to: uv, add: shift });
                }
            }
        }
    }

    /// Begin dragging a new guide out of a ruler: down the layout from the
    /// left ruler, across it from the top one.
    fn begin_guide(&mut self, across: bool, at: egui::Pos2, xform: &Xform, part: &Part, layout: &Layout, labels: &[Decal], canvas: egui::Rect) {
        if !self.prefs.guides {
            self.prefs.guides = true;
        }
        let uv = xform.to_layout(at);
        self.targets = self.gather_targets(part, layout, labels, &[], &[], None, xform, canvas);
        self.gesture = Some(Gesture::Guide { index: None, across, at: if across { uv.x } else { uv.y }, removing: true });
    }

    fn snapping(&self) -> bool {
        self.prefs.snapping && !self.modifiers.command
    }

    fn snap_point(&self, at: Vec2, xform: &Xform) -> snapping::Snap {
        if !self.snapping() {
            return snapping::Snap::none();
        }
        self.targets.snap_point(at, xform.layout_length(snapping::PIXELS))
    }

    /// Carry the drag on to the pointer at `at` on the screen; `over_ruler`
    /// when it is over one of the rulers, where a guide let go is removed.
    fn update(&mut self, at: egui::Pos2, over_ruler: bool, xform: &Xform) {
        let uv = xform.to_layout(at);
        let tolerance = xform.layout_length(snapping::PIXELS);
        let snapping = self.snapping();
        let (shift, alt) = (self.modifiers.shift, self.modifiers.alt);
        let mut snap = snapping::Snap::none();
        let targets = std::mem::take(&mut self.targets);
        match &mut self.gesture {
            Some(Gesture::Transform { grab, from, frame, original, current }) => {
                let frame = *frame;
                let single = original.len() == 1;
                let originals = original.clone();
                *current = match *grab {
                    Grab::Move => {
                        let boxes: Vec<_> = originals.iter().map(|(_, m)| placement::bounds(m)).collect();
                        let (delta, hit) = match super::align::union(&boxes) {
                            Some(bounds) => editing::snapped_move(&targets, bounds, uv - *from, shift, snapping, tolerance),
                            None => (uv - *from, snapping::Snap::none()),
                        };
                        snap = hit;
                        originals
                            .iter()
                            .map(|(item, m)| {
                                (*item, match item {
                                    Item::Texture(_) => placement::placed(*m, vec2(0.5, 0.5), placement::centre(m) + delta),
                                    _ => placement::moved(*m, delta),
                                })
                            })
                            .collect()
                    }
                    Grab::Handle { across, down } => {
                        let pointer = if snapping {
                            snap = targets.snap_point(uv, tolerance);
                            uv + snap.delta
                        } else {
                            uv
                        };
                        let (sx, sy) = (across as f32, down as f32);
                        let anchor = if alt { frame.centre } else { frame.handle(-sx, -sy) };
                        let reach = frame.local(frame.handle(sx, sy)) - frame.local(anchor);
                        let now = frame.local(pointer) - frame.local(anchor);
                        let mut k = vec2(if across != 0 { now.x / reach.x } else { 1.0 }, if down != 0 { now.y / reach.y } else { 1.0 });
                        // A corner keeps the shape, unless Shift frees it.
                        if across != 0 && down != 0 && !shift {
                            let along = (now.x * reach.x + now.y * reach.y) / (reach.x * reach.x + reach.y * reach.y).max(1e-12);
                            k = Vec2::splat(along);
                        }
                        let k = k.max(Vec2::splat(0.02));
                        scaled(&originals, single, frame, anchor, k)
                    }
                    Grab::Scale => {
                        let pivot = frame.centre;
                        let (a, b) = (*from - pivot, uv - pivot);
                        let k = if shift {
                            vec2(if a.x.abs() > 1e-6 { (b.x / a.x).abs() } else { 1.0 }, if a.y.abs() > 1e-6 { (b.y / a.y).abs() } else { 1.0 })
                        } else {
                            Vec2::splat(b.length() / a.length().max(1e-6))
                        };
                        scaled(&originals, single, frame, pivot, k.max(Vec2::splat(0.02)))
                    }
                    Grab::Turn => {
                        let pivot = frame.centre;
                        let angle = |p: Vec2| (p.y - pivot.y).atan2(p.x - pivot.x);
                        let mut turn = angle(uv) - angle(*from);
                        if single {
                            let mut to = frame.angle + turn;
                            if shift {
                                to = snapping::snap_angle(to, TURN_STEP);
                            }
                            turn = to - frame.angle;
                        } else if shift {
                            turn = snapping::snap_angle(turn, TURN_STEP);
                        }
                        originals
                            .iter()
                            .map(|(item, m)| {
                                (*item, match item {
                                    Item::Label(_) if single => placement::turned(*m, frame.angle + turn),
                                    Item::Texture(_) => placement::tile_turned(*m, turn),
                                    _ => placement::turned_about(*m, pivot, turn),
                                })
                            })
                            .collect()
                    }
                };
            }
            Some(Gesture::Layout { grab, from, frame, current, .. }) => {
                let frame = *frame;
                *current = match *grab {
                    Grab::Move => {
                        let bounds = (frame.centre - frame.size * 0.5, frame.centre + frame.size * 0.5);
                        let (delta, hit) = editing::snapped_move(&targets, bounds, uv - *from, shift, snapping, tolerance);
                        snap = hit;
                        editing::moved(delta)
                    }
                    Grab::Handle { across, down } => {
                        let pointer = if snapping {
                            snap = targets.snap_point(uv, tolerance);
                            uv + snap.delta
                        } else {
                            uv
                        };
                        editing::handle_scale(&frame, across, down, pointer, shift, alt)
                    }
                    Grab::Scale => editing::tool_scale(&frame, *from, uv, shift),
                    Grab::Turn => editing::tool_turn(&frame, *from, uv, shift.then_some(TURN_STEP)).0,
                };
            }
            Some(Gesture::Box { to, .. }) => *to = uv,
            Some(Gesture::Guide { across, at, removing, .. }) => {
                let raw = if *across { uv.x } else { uv.y };
                *at = raw;
                if snapping {
                    snap = targets.snap_line(*across, raw, tolerance);
                    *at = raw + if *across { snap.delta.x } else { snap.delta.y };
                }
                *removing = over_ruler;
            }
            Some(Gesture::Measure { from, to }) => {
                *to = if shift {
                    snapping::constrained(*from, uv)
                } else if snapping {
                    snap = targets.snap_point(uv, tolerance);
                    uv + snap.delta
                } else {
                    uv
                };
            }
            None => {}
        }
        self.targets = targets;
        self.snap = snap;
    }

    /// The drag let go, with the layout at hand: a drag of the layout's own
    /// becomes one `uv.transform`, a box picks what the mode picks; the
    /// rest as `finish` makes them.
    fn finish_with(&mut self, labels: &[Decal], part: &Part, layout: &Layout, edits: &mut Edits, scale: super::units::Scale) {
        match self.gesture.take() {
            Some(Gesture::Layout { grab, current, .. }) => {
                self.snap = snapping::Snap::none();
                self.targets = Targets::new();
                let done = match grab {
                    Grab::Move => "Moved",
                    Grab::Turn => "Turned",
                    Grab::Scale | Grab::Handle { .. } => "Scaled",
                };
                if current != UvAffine::IDENTITY {
                    self.transform_layout(part, layout, current, edits, done);
                }
            }
            Some(Gesture::Box { from, to, add }) if self.mode.is_layout() => {
                self.snap = snapping::Snap::none();
                self.targets = Targets::new();
                let picked = editing::boxed(layout, self.mode, from, to);
                if add {
                    self.selection.add_all(picked);
                } else {
                    self.selection.set_all(picked);
                }
            }
            other => {
                self.gesture = other;
                self.finish(labels, part, edits, scale);
            }
        }
    }

    /// The drag let go: what it did becomes an edit.
    pub(super) fn finish(&mut self, labels: &[Decal], part: &Part, edits: &mut Edits, scale: super::units::Scale) {
        self.snap = snapping::Snap::none();
        self.targets = Targets::new();
        match self.gesture.take() {
            Some(Gesture::Transform { original, current, .. }) => {
                if original != current {
                    for (item, mapping) in current {
                        match item {
                            Item::Label(i) => {
                                if let Some(label) = edits.labels_mut(part).get_mut(i) {
                                    label.mapping = mapping;
                                }
                            }
                            Item::Texture(param) => edits.textures.push((param, mapping)),
                            _ => {}
                        }
                    }
                }
            }
            Some(Gesture::Box { from, to, add }) => {
                let area = layout::rect(from, to);
                if !add {
                    self.selection.clear();
                }
                for (index, label) in labels.iter().enumerate() {
                    if pickable(label) && layout::overlaps(area, placement::bounds(&label.mapping)) {
                        self.selection.add(Item::Label(index));
                    }
                }
            }
            Some(Gesture::Guide { index, across, at, removing }) => {
                let mut guides = edits.guides.clone().unwrap_or_else(|| part.guides.clone());
                let axis = if across { GuideAxis::U } else { GuideAxis::V };
                match (index, removing) {
                    (Some(i), true) if i < guides.len() => {
                        guides.remove(i);
                        self.selected_guide = None;
                        edits.status = Some("Removed the guide".into());
                    }
                    (Some(i), false) if i < guides.len() => guides[i] = UvGuide::new(axis, at),
                    (None, false) => {
                        guides.push(UvGuide::new(axis, at));
                        self.selected_guide = Some(guides.len() - 1);
                        edits.status = Some(format!("Guide at {}", scale.format(at)));
                    }
                    _ => return,
                }
                if guides != part.guides {
                    edits.guides = Some(guides);
                }
            }
            Some(Gesture::Measure { from, to }) => {
                self.measure = ((to - from).length() > 1e-7).then_some((from, to));
            }
            Some(Gesture::Layout { .. }) | None => {}
        }
    }

    /// A click without a drag.
    fn click(&mut self, at: egui::Pos2, xform: &Xform, labels: &[Decal], part: &Part, layout: &Layout) {
        let shift = self.modifiers.shift;
        match self.pick_any(at, xform, labels, part, layout) {
            Hit::Handle(_) => {}
            Hit::Item(item) => {
                self.selected_guide = None;
                if shift {
                    self.selection.toggle(item);
                } else {
                    self.selection.set(item);
                }
                if let Item::Island(t) = item {
                    self.island = layout.uv.island_of.get(t as usize).map(|&i| i as usize);
                }
            }
            Hit::Guide(index) => self.selected_guide = Some(index),
            Hit::Empty => {
                self.selected_guide = None;
                if self.tool == Tool::Measure {
                    return;
                }
                if !shift {
                    self.selection.clear();
                }
                self.island = layout.at(xform.to_layout(at));
            }
        }
    }

    /// The canvas, with its rulers: drawn into what is left of `ui`.
    pub(super) fn canvas(&mut self, ui: &mut egui::Ui, part: &Part, layout: &Layout, edits: &mut Edits, project: &crate::project::Project) {
        let full = ui.available_rect_before_wrap();
        ui.allocate_rect(full, egui::Sense::hover());
        let ruler = if self.prefs.rulers { rulers::THICKNESS } else { 0.0 };
        let canvas = egui::Rect::from_min_max(full.min + egui::vec2(ruler, ruler), full.max);
        let top = egui::Rect::from_min_max(egui::pos2(canvas.left(), full.top()), egui::pos2(full.right(), canvas.top()));
        let left = egui::Rect::from_min_max(egui::pos2(full.left(), canvas.top()), egui::pos2(canvas.left(), full.bottom()));
        let corner = egui::Rect::from_min_max(full.min, canvas.min);
        self.canvas_rect = canvas;

        let response = ui.interact(canvas, ui.id().with("uv-canvas"), egui::Sense::click_and_drag());
        let (top_response, left_response) = if self.prefs.rulers {
            (
                Some(ui.interact(top, ui.id().with("uv-ruler-top"), egui::Sense::click_and_drag())),
                Some(ui.interact(left, ui.id().with("uv-ruler-left"), egui::Sense::click_and_drag())),
            )
        } else {
            (None, None)
        };
        self.modifiers = ui.input(|i| i.modifiers);
        if self.frame_pending {
            self.frame_pending = false;
            self.frame_all(labels_of(edits, part), part, layout);
        }
        if let Some(framing) = self.frame_request.take() {
            self.view.frame(canvas, framing.0, framing.1);
        }

        // ---- zoom and pan --------------------------------------------------
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(pointer) = response.hover_pos() {
                    self.view.zoom_about(canvas, 1.0 + scroll * 0.0015, pointer);
                }
            }
            let pinch = ui.input(|i| i.zoom_delta());
            if pinch != 1.0 {
                if let Some(pointer) = response.hover_pos() {
                    self.view.zoom_about(canvas, pinch, pointer);
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary) {
            self.view.pan += response.drag_delta();
        }
        let xform = self.view.xform(canvas);

        // Labels as edited this frame by the panels, before the canvas
        // draws them.
        let labels: Vec<Decal> = labels_of(edits, part).to_vec();

        // ---- the pointer ---------------------------------------------------
        let hover = ui.ctx().pointer_hover_pos().filter(|p| full.contains(*p));
        // A script's pointer, or where its held drag has the pointer now.
        let scripted_pointer = self.scripted.pointer.or(self.scripted.drag.map(|(_, to)| to)).map(|p| xform.to_screen(p));
        let pointer_screen = scripted_pointer.or(hover).or_else(|| ui.ctx().pointer_interact_pos().filter(|_| self.gesture.is_some()));
        self.pointer = pointer_screen.map(|p| xform.to_layout(p));
        self.hover = match (self.pointer, pointer_screen.is_some_and(|p| canvas.contains(p))) {
            (Some(at), true) => editing::Hover::at(layout, at, xform.layout_length(GRAB)),
            _ => editing::Hover::default(),
        };

        // ---- input: the drag ------------------------------------------------
        let primary_started = response.drag_started_by(egui::PointerButton::Primary);
        if primary_started {
            if let Some(at) = ui.input(|i| i.pointer.press_origin()) {
                self.begin(at, &xform, &labels, part, layout, canvas);
            }
        }
        for (ruler, across) in [(&top_response, false), (&left_response, true)] {
            if let Some(ruler) = ruler {
                if ruler.drag_started_by(egui::PointerButton::Primary) && self.gesture.is_none() {
                    if let Some(at) = ui.input(|i| i.pointer.press_origin()) {
                        self.begin_guide(across, at, &xform, part, layout, &labels, canvas);
                    }
                }
            }
        }
        // A drag held for a script, from one point of the layout to another.
        if let Some((from, to)) = self.scripted.drag {
            if self.gesture.is_none() {
                self.begin(xform.to_screen(from), &xform, &labels, part, layout, canvas);
            }
            self.update(xform.to_screen(to), false, &xform);
            if self.scripted.release {
                self.scripted.drag = None;
                self.scripted.release = false;
                self.finish_with(&labels, part, layout, edits, self.scale);
            }
        } else if self.gesture.is_some() {
            let dragging = ui.input(|i| i.pointer.primary_down());
            if let Some(at) = ui.ctx().pointer_interact_pos() {
                let over_ruler = self.prefs.rulers && (top.contains(at) || left.contains(at) || corner.contains(at)) || !full.contains(at);
                self.update(at, over_ruler, &xform);
            }
            if !dragging {
                self.finish_with(&labels, part, layout, edits, self.scale);
            }
            ui.ctx().request_repaint();
        }
        if response.clicked() {
            if let Some(at) = response.interact_pointer_pos() {
                self.click(at, &xform, &labels, part, layout);
            }
        }
        if let Some(at) = self.scripted.click.take() {
            self.click(xform.to_screen(at), &xform, &labels, part, layout);
        }
        if response.double_clicked() {
            if let Some(at) = response.interact_pointer_pos() {
                if let Hit::Guide(index) = self.pick_any(at, &xform, &labels, part, layout) {
                    let guide = part.guides[index];
                    self.typing = Some(super::guides::Typing::new(index, guide, self.scale, at));
                }
            }
        }
        if response.secondary_clicked() {
            if let Some(at) = response.interact_pointer_pos() {
                if let Hit::Item(item) = self.pick_any(at, &xform, &labels, part, layout) {
                    if !self.selection.contains(item) {
                        self.selection.set(item);
                    }
                }
            }
        }
        let mut menu = None;
        response.context_menu(|ui| menu = context_menu(ui, self, part, layout));
        if let Some(action) = menu {
            self.menu_action(action, part, layout, edits);
        }

        // ---- drawing --------------------------------------------------------
        let painter = ui.painter_at(canvas);
        let p = crate::theme::p();
        let tint = colours();
        painter.rect_filled(canvas, 0.0, p.panel);
        let square = xform.rect(Vec2::ZERO, Vec2::splat(1.0));
        painter.rect_filled(square, 0.0, p.canvas);
        if self.prefs.grid {
            self.draw_grid(&painter, &xform, canvas, layout.density());
        }
        painter.rect_stroke(square, 0.0, egui::Stroke::new(1.0_f32, p.line_strong));

        // A texture being placed, under the labels: its tile, and the
        // outlines of the tiles round it.
        if let Some(param) = self.selection.texture() {
            if let (Some(mapping), Some(layer)) = (shown_mapping(self.gesture.as_ref(), Item::Texture(param), &labels, part), part.textures.iter().find(|t| t.param == param)) {
                let picture = layer.texture.and_then(|id| super::artwork(&mut self.artwork, project, id, ui.ctx()));
                let tile = placement::corners(&mapping);
                let step = (tile[1] - tile[0], tile[3] - tile[0]);
                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        let shift = step.0 * dx as f32 + step.1 * dy as f32;
                        let quad = tile.map(|c| xform.to_screen(c + shift));
                        let main = dx == 0 && dy == 0;
                        if let Some(picture) = &picture {
                            textured_quad(&painter, picture, quad, egui::Color32::WHITE.gamma_multiply(if main { 0.75 } else { 0.25 } * self.prefs.artwork_opacity));
                        }
                        outline(&painter, quad, egui::Stroke::new(1.0_f32, p.accent.gamma_multiply(if main { 0.9 } else { 0.3 })));
                    }
                }
            }
        }

        // Where the layout's own is drawn this frame: as it is, or through
        // a drag's map, or the Transform panel's while a field is held.
        let preview = match &self.gesture {
            Some(Gesture::Layout { current, moving, .. }) => Some((*current, moving.clone())),
            _ => self.panel_preview.map(|map| (map, editing::moving(layout, &self.selection))),
        };
        // The islands it carries whole, and the labels on them with them.
        let carried: HashSet<usize> = match (&preview, self.mode) {
            (Some(_), Mode::Islands) => editing::islands(layout, &self.selection).into_iter().map(|i| i as usize).collect(),
            _ => HashSet::new(),
        };

        // The labels, bottom first; under the layout's lines so they show
        // through. The others are dimmed while some are selected, and all
        // of them while the layout's own is worked on.
        let layout_work = self.mode.is_layout();
        let dim = !self.selection.is_empty();
        for (index, label) in labels.iter().enumerate() {
            if label.mapping.projection != Projection::Uv || !label.enabled || label.mapping.uv_set != layout.uv.set {
                continue;
            }
            let item = Item::Label(index);
            let mapping = shown_mapping(self.gesture.as_ref(), item, &labels, part).unwrap_or(label.mapping);
            let mut quad = placement::corners(&mapping).map(|c| xform.to_screen(c));
            if let (Some((map, _)), Some(island)) = (&preview, layout.at(placement::centre(&mapping))) {
                if carried.contains(&island) {
                    quad = placement::corners(&mapping).map(|c| xform.to_screen(map.apply(c)));
                }
            }
            let selected = self.selection.contains(item);
            let strength = self.prefs.artwork_opacity * label.opacity.max(0.15) * if (dim && !selected) || layout_work { 0.35 } else { 1.0 };
            if self.prefs.artwork {
                match super::artwork(&mut self.artwork, project, label.texture, ui.ctx()) {
                    Some(picture) => textured_quad(&painter, &picture, quad, egui::Color32::WHITE.gamma_multiply(strength)),
                    None => fill_quad(&painter, quad, p.accent.gamma_multiply(0.3 * strength)),
                }
            }
            let stroke = if label.locked {
                egui::Stroke::new(1.0_f32, p.text_faint)
            } else {
                egui::Stroke::new(1.0_f32, p.text_dim.gamma_multiply(if (dim && !selected) || layout_work { 0.4 } else { 0.8 }))
            };
            outline(&painter, quad, stroke);
        }

        let places: Option<Vec<Vec2>> = preview.as_ref().map(|(map, moving)| {
            let mut out: Vec<Vec2> = layout.uv.uv_vertices.iter().map(|v| v.uv).collect();
            for &v in moving {
                out[v as usize] = map.apply(out[v as usize]);
            }
            out
        });
        let place = |v: u32| -> Vec2 {
            match &places {
                Some(all) => all[v as usize],
                None => layout.uv.uv_vertices[v as usize].uv,
            }
        };
        let corners = |t: u32| -> [Vec2; 3] {
            let base = t as usize * 3;
            [0, 1, 2].map(|k| place(layout.uv.corner_vertex[base + k]))
        };
        let fill = |triangles: &mut dyn Iterator<Item = u32>, colour: &dyn Fn(u32) -> egui::Color32| {
            let mut mesh = egui::Mesh::default();
            for t in triangles {
                let base = mesh.vertices.len() as u32;
                let c = colour(t);
                for corner in corners(t) {
                    mesh.colored_vertex(xform.to_screen(corner), c);
                }
                mesh.add_triangle(base, base + 1, base + 2);
            }
            painter.add(egui::Shape::mesh(mesh));
        };

        // The stretch heatmap, over the artwork and under the lines.
        if let (Some(measure), Some(distortion)) = (self.prefs.stretch, self.distortion_for(part, layout)) {
            let alpha = 0.82;
            fill(&mut (0..layout.triangles.len() as u32), &|t| {
                let [r, g, b] = distortion.get(t as usize).map(|d| tequila_scene::uv::heat(d, measure)).unwrap_or([255, 255, 255]);
                egui::Color32::from_rgb(r, g, b).gamma_multiply(alpha)
            });
        }

        // The islands: the one under the pointer, the selected, the one the
        // viewport points at, and the faces picked on the part.
        let selected_islands = editing::islands(layout, &self.selection);
        let hovered_island = match (self.hover.island, self.mode, &self.gesture) {
            (Some(island), Mode::Islands, None) => Some(island),
            _ => None,
        };
        if let Some(island) = hovered_island.filter(|i| !selected_islands.contains(i)) {
            fill(&mut layout.uv.islands[island as usize].triangles.iter().copied(), &|_| p.accent.gamma_multiply(0.14));
        }
        for &island in &selected_islands {
            fill(&mut layout.uv.islands[island as usize].triangles.iter().copied(), &|_| p.accent.gamma_multiply(0.24));
        }
        // The island last clicked, as labels are fitted to it.
        if let (Some(island), Mode::Labels) = (self.island.filter(|i| *i < layout.uv.islands.len()), self.mode) {
            fill(&mut layout.uv.islands[island].triangles.iter().copied(), &|_| p.accent.gamma_multiply(0.22));
        }
        if !part.faces.is_empty() {
            fill(&mut part.faces.iter().copied().filter(|&t| (t as usize) < layout.triangles.len()), &|_| tint.picked.gamma_multiply(0.38));
        }
        let linked = self.viewport_hover.filter(|h| h.node == part.node && (h.triangle as usize) < layout.triangles.len());
        if let Some(hover) = linked {
            let island = layout.uv.island_of[hover.triangle as usize];
            fill(&mut layout.uv.islands[island as usize].triangles.iter().copied(), &|_| tint.linked.gamma_multiply(0.16));
            fill(&mut std::iter::once(hover.triangle), &|_| tint.linked.gamma_multiply(0.55));
        }

        // Every edge, faint, when there are few enough to see; the part's
        // edge firm, and the seams in colour.
        let segment = |e: u32| layout.uv.edges[e as usize].ends.map(|v| xform.to_screen(place(v)));
        let inner = layout.uv.edges.iter().filter(|e| e.kind == EdgeKind::Inner).count();
        if self.prefs.edges && inner <= 120_000 {
            let stroke = egui::Stroke::new(0.6_f32, p.text_dim.gamma_multiply(0.45));
            for (e, edge) in layout.uv.edges.iter().enumerate() {
                if edge.kind == EdgeKind::Inner {
                    painter.line_segment(segment(e as u32), stroke);
                }
            }
        }
        let (boundary, seam) = (egui::Stroke::new(1.2_f32, p.text), egui::Stroke::new(1.6_f32, tint.seam));
        for (e, edge) in layout.uv.edges.iter().enumerate() {
            match edge.kind {
                EdgeKind::Boundary => {
                    painter.line_segment(segment(e as u32), boundary);
                }
                EdgeKind::Seam => {
                    painter.line_segment(segment(e as u32), seam);
                }
                EdgeKind::Inner => {}
            }
        }
        // The island under the pointer, outlined, and the selected ones.
        let outline_of = |island: u32, stroke: egui::Stroke| {
            for e in layout.uv.islands[island as usize].edges.clone() {
                if layout.uv.edges[e].kind != EdgeKind::Inner {
                    painter.line_segment(segment(e as u32), stroke);
                }
            }
        };
        for &island in &selected_islands {
            outline_of(island, egui::Stroke::new(2.0_f32, p.accent));
        }
        if let Some(island) = hovered_island {
            outline_of(island, egui::Stroke::new(2.0_f32, p.accent.gamma_multiply(0.7)));
        }
        // The selected edges, and the one under the pointer with, on a
        // seam, its other side.
        let chosen_edges = editing::selected_edges(layout, &self.selection);
        for &e in &chosen_edges {
            painter.line_segment(segment(e), egui::Stroke::new(3.0_f32, p.accent));
        }
        if let (Some(e), true, None) = (self.hover.edge, matches!(self.mode, Mode::Edges | Mode::Islands), &self.gesture) {
            let edge = &layout.uv.edges[e as usize];
            if self.mode == Mode::Edges || edge.kind == EdgeKind::Seam {
                // A seam: the whole of it between its two islands, on both
                // sides, so its other side is found wherever it lies, and
                // the edge under the pointer and its twin marked.
                if let Some(twin) = edge.twin {
                    let line = egui::Stroke::new(2.5_f32, tint.hover.gamma_multiply(0.75));
                    for e in seam_between(layout, edge.island, layout.uv.edges[twin as usize].island) {
                        painter.line_segment(segment(e), line);
                    }
                }
                let stroke = egui::Stroke::new(4.0_f32, tint.hover);
                let [a, b] = segment(e);
                painter.line_segment([a, b], stroke);
                painter.circle_stroke(a + (b - a) * 0.5, 6.0, egui::Stroke::new(2.0_f32, tint.hover));
                if let Some(twin) = edge.twin {
                    let [a, b] = segment(twin);
                    painter.line_segment([a, b], stroke);
                    painter.circle_stroke(a + (b - a) * 0.5, 6.0, egui::Stroke::new(2.0_f32, tint.hover));
                }
            }
        }

        // Vertices: every one in the Vertices mode when they can be told
        // apart, the selected, the one under the pointer, and the pins.
        if self.mode == Mode::Vertices && layout.typical_edge * xform.scale >= DOTS_AT {
            let (low, high) = xform.visible(canvas);
            for v in 0..layout.uv.uv_vertices.len() as u32 {
                let at = place(v);
                if at.x >= low.x && at.y >= low.y && at.x <= high.x && at.y <= high.y {
                    painter.circle_filled(xform.to_screen(at), 1.8, p.text_dim);
                }
            }
        }
        for v in self.selection.vertices() {
            if let Some(&uv) = layout.vertex_uv.get(v as usize).filter(|&&u| u != u32::MAX) {
                let at = xform.to_screen(place(uv));
                let r = egui::Rect::from_center_size(at, egui::Vec2::splat(7.0));
                painter.rect_filled(r, 1.5, p.accent);
                painter.rect_stroke(r, 1.5, egui::Stroke::new(1.0_f32, p.on_accent));
            }
        }
        if let (Some(v), Mode::Vertices, None) = (self.hover.vertex, self.mode, &self.gesture) {
            painter.circle_stroke(xform.to_screen(place(v)), 6.0, egui::Stroke::new(2.0_f32, tint.hover));
        }
        if let Some(pins) = self.pins.get(&(part.node, layout.uv.set)) {
            for &v in pins {
                let Some(&uv) = layout.vertex_uv.get(v as usize).filter(|&&u| u != u32::MAX) else { continue };
                // A dot on the vertex, and a pin standing on it, on a plate
                // so it reads over the layout's lines.
                let at = xform.to_screen(place(uv));
                let head = at + egui::vec2(0.0, -17.0);
                painter.line_segment([at, head + egui::vec2(0.0, 8.0)], egui::Stroke::new(1.5_f32, tint.pin));
                painter.circle_filled(head, 10.5, p.raised);
                painter.circle_stroke(head, 10.5, egui::Stroke::new(1.5_f32, tint.pin));
                crate::icons::paint(&painter, egui::Rect::from_center_size(head, egui::Vec2::splat(14.0)), crate::icons::Icon::Pin, tint.pin);
                painter.circle_filled(at, 4.0, tint.pin);
                painter.circle_stroke(at, 4.0, egui::Stroke::new(1.0_f32, p.raised));
            }
        }
        if let Some(hover) = linked {
            let at = super::link::on_layout(corners(hover.triangle), hover.weights);
            let s = xform.to_screen(at);
            let stroke = egui::Stroke::new(1.5_f32, tint.linked);
            painter.circle_stroke(s, 6.0, stroke);
            painter.line_segment([s - egui::vec2(10.0, 0.0), s + egui::vec2(10.0, 0.0)], stroke);
            painter.line_segment([s - egui::vec2(0.0, 10.0), s + egui::vec2(0.0, 10.0)], stroke);
        }

        // Guides, over the layout.
        let guide_colour = crate::theme::hue(crate::theme::Hue::Sky);
        if self.prefs.guides {
            let dragged = match &self.gesture {
                Some(Gesture::Guide { index, .. }) => *index,
                _ => None,
            };
            for (index, guide) in edits.guides.as_ref().unwrap_or(&part.guides).iter().enumerate() {
                if Some(index) == dragged {
                    continue;
                }
                let strength = if self.prefs.guides_locked { 0.45 } else { 0.85 };
                let width = if self.selected_guide == Some(index) { 2.0_f32 } else { 1.0 };
                guide_line(&painter, canvas, &xform, guide.axis == GuideAxis::U, guide.at, egui::Stroke::new(width, guide_colour.gamma_multiply(strength)));
            }
        }

        // The selection's frame and its handles.
        if self.tool != Tool::Measure {
            if let Some(frame) = self.frame_now(&labels, part, layout) {
                let texture = self.selection.texture().is_some();
                let quad = frame.corners().map(|c| xform.to_screen(c));
                outline(&painter, quad, egui::Stroke::new(1.5_f32, p.accent));
                let handles = self.handles(&frame, &xform, texture);
                let top = xform.to_screen(frame.handle(0.0, -1.0));
                for (grab, place) in handles {
                    match grab {
                        Grab::Turn => {
                            painter.line_segment([top, place], egui::Stroke::new(1.5_f32, p.accent));
                            painter.circle_filled(place, 5.5, p.accent);
                            painter.circle_stroke(place, 5.5, egui::Stroke::new(1.0_f32, p.on_accent));
                        }
                        _ => {
                            let r = egui::Rect::from_center_size(place, egui::Vec2::splat(9.0));
                            painter.rect_filled(r, 2.0, p.on_accent);
                            painter.rect_stroke(r, 2.0, egui::Stroke::new(1.5_f32, p.accent));
                        }
                    }
                }
            }
        }

        // What a drag is doing: the box, the guide, the measure, and what it
        // snapped to.
        match &self.gesture {
            Some(Gesture::Box { from, to, .. }) => {
                let r = xform.rect(*from, *to);
                painter.rect_filled(r, 0.0, p.accent.gamma_multiply(0.08));
                painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0_f32, p.accent));
            }
            Some(Gesture::Guide { across, at, removing, .. }) if !*removing => {
                guide_line(&painter, canvas, &xform, *across, *at, egui::Stroke::new(1.5_f32, guide_colour));
                let place = if *across { xform.to_screen(vec2(*at, 0.0)).x } else { xform.to_screen(vec2(0.0, *at)).y };
                let tag = egui::pos2(if *across { place + 6.0 } else { canvas.left() + 8.0 }, if *across { canvas.top() + 8.0 } else { place + 4.0 });
                note(&painter, tag, &self.scale.format(*at));
            }
            Some(Gesture::Measure { from, to }) => draw_measure(&painter, &xform, *from, *to, self.scale),
            Some(Gesture::Layout { grab: Grab::Turn, current, .. }) => {
                if let Some(at) = pointer_screen {
                    let turn = -current.m[1][0].atan2(current.m[0][0]).to_degrees();
                    note(&painter, at + egui::vec2(14.0, -26.0), &format!("{turn:.1}°"));
                }
            }
            _ => {}
        }
        // The last measure stays, under whatever is dragged, until the next.
        if !matches!(self.gesture, Some(Gesture::Measure { .. })) {
            if let Some((from, to)) = self.measure {
                draw_measure(&painter, &xform, from, to, self.scale);
            }
        }
        if self.snap.is_some() {
            self.draw_snap(&painter, &xform, canvas, pointer_screen);
        }
        if let Some(measure) = self.prefs.stretch {
            legend(&painter, canvas, measure);
        }

        // ---- the rulers -----------------------------------------------------
        if self.prefs.rulers {
            let ruler_painter = ui.painter_at(full);
            rulers::draw(&ruler_painter, top, &xform, self.scale, true, self.pointer.map(|p| p.x));
            rulers::draw(&ruler_painter, left, &xform, self.scale, false, self.pointer.map(|p| p.y));
            ruler_painter.rect_filled(corner, 0.0, p.chrome);
            ruler_painter.text(corner.center(), egui::Align2::CENTER_CENTER, self.scale.unit.suffix(), egui::FontId::proportional(9.5), p.text_dim);
            let corner_response = ui.interact(corner, ui.id().with("uv-ruler-corner"), egui::Sense::click());
            let corner_response = corner_response.on_hover_text("The unit the rulers read in. Click for the next one.");
            if corner_response.clicked() {
                if let Some(note) = self.next_unit(layout) {
                    edits.status = Some(note);
                }
            }
            for ruler in [&top_response, &left_response].into_iter().flatten() {
                if ruler.hovered() && self.gesture.is_none() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }
            }
            if let Some(r) = &left_response {
                if r.hovered() && self.gesture.is_none() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            }
            if let Some(r) = &top_response {
                let _ = r.clone().on_hover_text("Drag down onto the layout for a guide across it");
            }
            if let Some(r) = &left_response {
                let _ = r.clone().on_hover_text("Drag across onto the layout for a guide down it");
            }
        }
        // The cursor over what can be grabbed.
        if let (Some(at), None) = (hover.filter(|p| canvas.contains(*p)), &self.gesture) {
            match self.pick_any(at, &xform, &labels, part, layout) {
                Hit::Handle(Grab::Turn) => ui.ctx().set_cursor_icon(egui::CursorIcon::Alias),
                Hit::Handle(_) => ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair),
                Hit::Item(_) if self.tool != Tool::Measure => ui.ctx().set_cursor_icon(if self.mode.is_layout() { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Move }),
                Hit::Guide(index) => ui.ctx().set_cursor_icon(if part.guides[index].axis == GuideAxis::U { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical }),
                _ if self.tool == Tool::Measure => ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair),
                _ => {}
            }
        }
        self.hint = self.viewport_hint_of(part, layout);
    }

    /// What the viewport lights for what is under the pointer here: the
    /// island, the face, the edge and its seam's other side, the vertex.
    fn viewport_hint_of(&self, part: &Part, layout: &Layout) -> super::link::Hint {
        let mut hint = super::link::Hint { node: Some(part.node), ..Default::default() };
        if self.gesture.is_some() {
            return hint;
        }
        let outline = |island: u32| -> Vec<[u32; 2]> { layout.uv.edges[layout.uv.islands[island as usize].edges.clone()].iter().filter(|e| e.kind != EdgeKind::Inner).map(|e| e.vertices).collect() };
        match self.mode {
            Mode::Islands => {
                if let Some(island) = self.hover.island {
                    hint.triangles = layout.uv.islands[island as usize].triangles.clone();
                    hint.edges = outline(island);
                }
            }
            Mode::Edges => {
                if let Some(e) = self.hover.edge {
                    let edge = &layout.uv.edges[e as usize];
                    hint.edges.push(edge.vertices);
                    // A seam on the part is one line, both its sides.
                    if let Some(twin) = edge.twin {
                        hint.edges.extend(seam_between(layout, edge.island, layout.uv.edges[twin as usize].island).into_iter().map(|e| layout.uv.edges[e as usize].vertices));
                    }
                    if let Some(t) = self.hover.triangle {
                        hint.triangles.push(t);
                    }
                }
            }
            Mode::Vertices => {
                if let Some(v) = self.hover.vertex {
                    hint.vertices.push(layout.uv.uv_vertices[v as usize].vertex);
                }
            }
            Mode::Labels => {
                if let Some(t) = self.hover.triangle {
                    hint.triangles.push(t);
                    if let Some(island) = self.hover.island {
                        hint.edges = outline(island);
                    }
                }
            }
        }
        hint
    }

    fn draw_grid(&self, painter: &egui::Painter, xform: &Xform, canvas: egui::Rect, density: Option<f32>) {
        let p = crate::theme::p();
        let (major, minor) = self.grid_steps(density);
        let (low, high) = xform.visible(canvas);
        let lines = |step: Vec2, stroke: egui::Stroke, skip_major: bool| {
            if step.x * xform.scale < 6.0 {
                return;
            }
            let (first, last) = ((low.x / step.x).floor() as i64, (high.x / step.x).ceil() as i64);
            if last - first > 600 {
                return;
            }
            for k in first..=last {
                let u = k as f32 * step.x;
                if skip_major && ((u / major.x).round() * major.x - u).abs() < step.x * 1e-3 {
                    continue;
                }
                let x = xform.to_screen(vec2(u, 0.0)).x;
                painter.line_segment([egui::pos2(x, canvas.top()), egui::pos2(x, canvas.bottom())], stroke);
            }
            let (first, last) = ((low.y / step.y).floor() as i64, (high.y / step.y).ceil() as i64);
            for k in first..=last {
                let v = k as f32 * step.y;
                if skip_major && ((v / major.y).round() * major.y - v).abs() < step.y * 1e-3 {
                    continue;
                }
                let y = xform.to_screen(vec2(0.0, v)).y;
                painter.line_segment([egui::pos2(canvas.left(), y), egui::pos2(canvas.right(), y)], stroke);
            }
        };
        if self.prefs.subdivisions > 1 {
            lines(minor, egui::Stroke::new(1.0_f32, p.line.gamma_multiply(0.35)), true);
        }
        lines(major, egui::Stroke::new(1.0_f32, p.line.gamma_multiply(0.8)), false);
    }

    /// What the drag snapped to, highlighted, with a note by the pointer.
    fn draw_snap(&self, painter: &egui::Painter, xform: &Xform, canvas: egui::Rect, pointer: Option<egui::Pos2>) {
        let colour = crate::theme::hue(crate::theme::Hue::Pink);
        let stroke = egui::Stroke::new(1.5_f32, colour);
        // A line with an extent is drawn over it and a little past; a guide
        // or the grid, across the canvas.
        if let Some(hit) = self.snap.across {
            let x = xform.to_screen(vec2(hit.line.at, 0.0)).x;
            let (a, b) = if hit.line.from.is_finite() {
                (xform.to_screen(vec2(0.0, hit.line.from)).y - 12.0, xform.to_screen(vec2(0.0, hit.line.to)).y + 12.0)
            } else {
                (canvas.top(), canvas.bottom())
            };
            dashed(painter, egui::pos2(x, a.max(canvas.top())), egui::pos2(x, b.min(canvas.bottom())), stroke);
        }
        if let Some(hit) = self.snap.down {
            let y = xform.to_screen(vec2(0.0, hit.line.at)).y;
            let (a, b) = if hit.line.from.is_finite() {
                (xform.to_screen(vec2(hit.line.from, 0.0)).x - 12.0, xform.to_screen(vec2(hit.line.to, 0.0)).x + 12.0)
            } else {
                (canvas.left(), canvas.right())
            };
            dashed(painter, egui::pos2(a.max(canvas.left()), y), egui::pos2(b.min(canvas.right()), y), stroke);
        }
        if let Some(point) = self.snap.point {
            let at = xform.to_screen(point.at);
            painter.circle_stroke(at, 6.0, egui::Stroke::new(1.5_f32, colour));
            painter.line_segment([at - egui::vec2(4.0, 0.0), at + egui::vec2(4.0, 0.0)], stroke);
            painter.line_segment([at - egui::vec2(0.0, 4.0), at + egui::vec2(0.0, 4.0)], stroke);
        }
        if let (Some(text), Some(at)) = (self.snap.note(), pointer) {
            note(painter, at + egui::vec2(14.0, 14.0), text);
        }
    }

    /// Frame everything: the islands and the labels on the layout.
    pub(super) fn frame_all(&mut self, labels: &[Decal], _part: &Part, layout: &Layout) {
        let mut boxes: Vec<(Vec2, Vec2)> = layout.extent().into_iter().collect();
        boxes.push((Vec2::ZERO, Vec2::splat(1.0)));
        boxes.extend(labels.iter().filter(|l| l.mapping.projection == Projection::Uv && l.enabled && l.mapping.uv_set == layout.uv.set).map(|l| placement::bounds(&l.mapping)));
        if let Some(all) = super::align::union(&boxes) {
            self.frame_request = Some(all);
        }
    }

    /// Frame the selection, or everything when nothing is selected.
    pub(super) fn frame_selection(&mut self, labels: &[Decal], part: &Part, layout: &Layout) {
        if self.selection.is_layout() {
            if let Some(frame) = self.layout_frame(layout) {
                let pad = frame.size.max(Vec2::splat(1e-3)) * 0.05;
                self.frame_request = Some((frame.centre - frame.size * 0.5 - pad, frame.centre + frame.size * 0.5 + pad));
                return;
            }
        }
        let boxes: Vec<(Vec2, Vec2)> = self
            .selection
            .items()
            .iter()
            .filter_map(|item| shown_mapping(None, *item, labels, part))
            .filter(|m| m.projection == Projection::Uv)
            .map(|m| placement::bounds(&m))
            .collect();
        match super::align::union(&boxes) {
            Some(all) => self.frame_request = Some(all),
            None => match self.island.and_then(|i| layout.boxes.get(i)) {
                Some(island) => self.frame_request = Some(*island),
                None => self.frame_all(labels, part, layout),
            },
        }
    }
}

/// The seam between two islands, both its sides: every seam edge of either
/// whose other side is on the other.
pub fn seam_between(layout: &Layout, a: u32, b: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for (island, other) in [(a, b), (b, a)] {
        for e in layout.uv.islands[island as usize].edges.clone() {
            let edge = &layout.uv.edges[e];
            if edge.kind == EdgeKind::Seam && edge.twin.is_some_and(|t| layout.uv.edges[t as usize].island == other) {
                out.push(e as u32);
            }
        }
    }
    out
}

/// The labels as edited so far this frame, or as the document has them.
pub fn labels_of<'a>(edits: &'a Edits, part: &'a Part) -> &'a [Decal] {
    edits.labels.as_deref().unwrap_or(&part.labels)
}

/// The scaled copies of what is being transformed: one label or texture in
/// its own frame, about `anchor`; several labels about it on the layout.
fn scaled(originals: &[(Item, Mapping)], single: bool, frame: Frame, anchor: Vec2, k: Vec2) -> Vec<(Item, Mapping)> {
    originals
        .iter()
        .map(|(item, m)| {
            let mapping = if single {
                let (x, y) = frame.axes();
                let offset = frame.local(frame.centre) - frame.local(anchor);
                let centre = anchor + x * (offset.x * k.x) + y * (offset.y * k.y);
                let size = frame.size * k;
                match item {
                    Item::Texture(_) => placement::placed(placement::with_repeats(*m, vec2(1.0 / size.x.max(placement::SMALLEST), 1.0 / size.y.max(placement::SMALLEST))), vec2(0.5, 0.5), centre),
                    _ => super::transform::apply_one(*m, Frame { centre, size, angle: frame.angle }),
                }
            } else {
                placement::scaled_about(*m, anchor, k)
            };
            (*item, mapping)
        })
        .collect()
}

/// The artwork stretched over a quad, its top left corner first.
fn textured_quad(painter: &egui::Painter, texture: &egui::TextureHandle, quad: [egui::Pos2; 4], tint: egui::Color32) {
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for (corner, uv) in quad.iter().zip([egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.0), egui::pos2(1.0, 1.0), egui::pos2(0.0, 1.0)]) {
        mesh.vertices.push(egui::epaint::Vertex { pos: *corner, uv, color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

fn fill_quad(painter: &egui::Painter, quad: [egui::Pos2; 4], colour: egui::Color32) {
    let mut mesh = egui::Mesh::default();
    for corner in quad {
        mesh.colored_vertex(corner, colour);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

fn outline(painter: &egui::Painter, quad: [egui::Pos2; 4], stroke: egui::Stroke) {
    painter.add(egui::Shape::closed_line(quad.to_vec(), stroke));
}

fn guide_line(painter: &egui::Painter, canvas: egui::Rect, xform: &Xform, across: bool, at: f32, stroke: egui::Stroke) {
    if across {
        let x = xform.to_screen(vec2(at, 0.0)).x;
        painter.line_segment([egui::pos2(x, canvas.top()), egui::pos2(x, canvas.bottom())], stroke);
    } else {
        let y = xform.to_screen(vec2(0.0, at)).y;
        painter.line_segment([egui::pos2(canvas.left(), y), egui::pos2(canvas.right(), y)], stroke);
    }
}

fn dashed(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2, stroke: egui::Stroke) {
    painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 5.0, 3.0));
}

/// A small note on a plate, as the snap's and a guide's place are shown.
pub fn note(painter: &egui::Painter, at: egui::Pos2, text: &str) {
    let p = crate::theme::p();
    let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::proportional(11.0), p.text);
    let rect = egui::Rect::from_min_size(at, galley.size() + egui::vec2(10.0, 4.0));
    painter.rect(rect, 4.0, p.raised, egui::Stroke::new(1.0_f32, p.line));
    painter.galley(rect.min + egui::vec2(5.0, 2.0), galley, p.text);
}

/// The heatmap's key, in the canvas's bottom left corner: its colours from
/// one end to the other, and what they mean.
fn legend(painter: &egui::Painter, canvas: egui::Rect, measure: Measure) {
    let p = crate::theme::p();
    let plate = egui::Rect::from_min_size(egui::pos2(canvas.left() + 12.0, canvas.bottom() - 64.0), egui::vec2(236.0, 52.0));
    painter.rect(plate, 6.0, p.raised.gamma_multiply(0.95), egui::Stroke::new(1.0_f32, p.line));
    let title = match measure {
        Measure::Area => "Stretch by area",
        Measure::Angle => "Stretch by angle",
    };
    painter.text(plate.min + egui::vec2(10.0, 7.0), egui::Align2::LEFT_TOP, title, egui::FontId::proportional(11.5), p.text);
    let bar = egui::Rect::from_min_size(plate.min + egui::vec2(10.0, 24.0), egui::vec2(plate.width() - 20.0, 9.0));
    let steps = 48;
    let mut mesh = egui::Mesh::default();
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let distortion = match measure {
            // A quarter to four times as dense, white in the middle.
            Measure::Area => tequila_scene::uv::Distortion { area: 2f32.powf(t * 4.0 - 2.0), angle: 1.0 },
            Measure::Angle => tequila_scene::uv::Distortion { area: 1.0, angle: 1.0 + t },
        };
        let [r, g, b] = tequila_scene::uv::heat(&distortion, measure);
        let colour = egui::Color32::from_rgb(r, g, b);
        let x = bar.left() + bar.width() * t;
        mesh.colored_vertex(egui::pos2(x, bar.top()), colour);
        mesh.colored_vertex(egui::pos2(x, bar.bottom()), colour);
        if i > 0 {
            let k = (i as u32) * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.rect_stroke(bar, 0.0, egui::Stroke::new(1.0_f32, p.line_strong));
    let (low, middle, high) = match measure {
        Measure::Area => ("¼× coarser", "even", "4× finer"),
        Measure::Angle => ("true", "1.5×", "2× squashed"),
    };
    let font = egui::FontId::proportional(10.0);
    let y = bar.bottom() + 3.0;
    painter.text(egui::pos2(bar.left(), y), egui::Align2::LEFT_TOP, low, font.clone(), p.text_dim);
    painter.text(egui::pos2(bar.center().x, y), egui::Align2::CENTER_TOP, middle, font.clone(), p.text_dim);
    painter.text(egui::pos2(bar.right(), y), egui::Align2::RIGHT_TOP, high, font, p.text_dim);
}

/// A dimension line from `from` to `to`, with its length on the part and
/// its angle.
fn draw_measure(painter: &egui::Painter, xform: &Xform, from: Vec2, to: Vec2, scale: super::units::Scale) {
    let colour = crate::theme::hue(crate::theme::Hue::Amber);
    let stroke = egui::Stroke::new(1.5_f32, colour);
    let (a, b) = (xform.to_screen(from), xform.to_screen(to));
    painter.line_segment([a, b], stroke);
    let along = (b - a).normalized();
    let across = egui::vec2(-along.y, along.x);
    if along.is_finite() {
        for end in [a, b] {
            painter.line_segment([end - across * 6.0, end + across * 6.0], stroke);
        }
        // Arrowheads, pointing out to the ends.
        for (end, way) in [(a, -along), (b, along)] {
            let back = end - way * 8.0;
            painter.add(egui::Shape::convex_polygon(vec![end, back + across * 3.5, back - across * 3.5], colour, egui::Stroke::NONE));
        }
    }
    let d = to - from;
    let degrees = -d.y.atan2(d.x).to_degrees();
    let text = format!("{} · {:.1}°", scale.format(d.length()), degrees);
    let middle = a + (b - a) * 0.5 + across * 14.0;
    note(painter, middle - egui::vec2(24.0, 8.0), &text);
}

/// The canvas's right-click menu: what can be done to the selection, the
/// guides and the view.
fn context_menu(ui: &mut egui::Ui, editor: &UvEditor, part: &Part, layout: &Layout) -> Option<MenuAction> {
    use super::actions::Action;
    let mut out = None;
    let mut item = |ui: &mut egui::Ui, enabled: bool, text: &str, keys: &str, action: MenuAction| {
        let button = egui::Button::new(text).shortcut_text(crate::theme::keys(keys));
        if ui.add_enabled(enabled, button).clicked() {
            out = Some(action);
            ui.close_menu();
        }
    };
    if editor.mode.is_layout() {
        let any = editor.selection.is_layout();
        let ready = |action: Action| editor.can(action, part, Some(layout)).is_ok();
        item(ui, ready(Action::Cut), "Cut", "C", MenuAction::Island(Action::Cut));
        item(ui, ready(Action::Stitch), "Stitch", "S", MenuAction::Island(Action::Stitch));
        ui.separator();
        item(ui, ready(Action::Pin), "Pin", "P", MenuAction::Island(Action::Pin));
        item(ui, ready(Action::Unpin), "Unpin", "Shift+P", MenuAction::Island(Action::Unpin));
        item(ui, true, "Re-flatten", "", MenuAction::Island(Action::Reflatten));
        ui.separator();
        item(ui, any, "Turn a quarter clockwise", "]", MenuAction::Quarter(1));
        item(ui, any, "Turn a quarter anticlockwise", "[", MenuAction::Quarter(-1));
        item(ui, any, "Flip horizontal", "Shift+H", MenuAction::Flip(true));
        item(ui, any, "Flip vertical", "Shift+V", MenuAction::Flip(false));
        ui.separator();
        item(ui, true, "Pack", "Ctrl+P", MenuAction::Island(Action::Pack));
        item(ui, true, "Straighten", "", MenuAction::Island(Action::Straighten));
        item(ui, true, "Even size", "", MenuAction::Island(Action::Even));
        ui.separator();
    } else {
        let labels = !editor.selection.labels().is_empty();
        item(ui, labels, "Duplicate", "Ctrl+D", MenuAction::Duplicate);
        item(ui, labels, "Delete", "Delete", MenuAction::Delete);
        ui.separator();
        item(ui, labels, "Bring to front", "", MenuAction::ToFront);
        item(ui, labels, "Send to back", "", MenuAction::ToBack);
        item(ui, labels, "Flip horizontal", "Shift+H", MenuAction::Flip(true));
        item(ui, labels, "Flip vertical", "Shift+V", MenuAction::Flip(false));
        ui.separator();
        let island = editor.island.is_some();
        item(ui, labels && island, "Fit to island", "", MenuAction::Fit);
        item(ui, labels && island, "Centre on island", "", MenuAction::Centre);
        item(ui, labels && island, "Along the island", "", MenuAction::Along);
        ui.separator();
    }
    item(ui, true, "Select all", "Ctrl+A", MenuAction::SelectAll);
    item(ui, true, "Frame selection", "F", MenuAction::FrameSelection);
    item(ui, true, "Frame all", "Home", MenuAction::FrameAll);
    ui.separator();
    item(ui, !part.guides.is_empty(), if editor.prefs.guides_locked { "Unlock guides" } else { "Lock guides" }, "", MenuAction::LockGuides);
    item(ui, !part.guides.is_empty(), "Clear guides", "", MenuAction::ClearGuides);
    out
}
