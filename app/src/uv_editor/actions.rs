//! The layout engine's edits, on the editor's selection: cutting and
//! stitching, pins and laying out flat again, packing, straightening,
//! evening the sizes, unwrapping again, moving and turning islands, the
//! part's UV sets, and the faces picked on the part in the viewport. Each
//! is one of the engine's commands (`tequila_scene::uv`), made at the end of
//! the frame with whatever else it changed as one step to undo; a refusal -
//! a cut that would fold, a pack with no room - says why in the status bar.
//!
//! And the Islands panel, which offers them all, with the set picker and
//! what the selection measures.

use super::editing;
use super::layout::Layout;
use super::selection::{Item, Mode};
use super::{Edits, Part, UvEditor};
use crate::icons::{self, Icon};
use tequila_core::math::vec2;
use tequila_scene::guides::GuideAxis;
use tequila_scene::uv::{Alignment, DensityTarget, PackOptions, StraightenBy, UvAffine, UvRect};
use tequila_scene::Command;

/// What the island tools do, as the toolbar, the panel and the keys offer
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Cut,
    Stitch,
    Pin,
    Unpin,
    Reflatten,
    Pack,
    Straighten,
    Even,
    Unwrap,
    FromFaces,
    CutFaces,
}

/// The island tools in the toolbar, after the modes.
pub const TOOLBAR: [Action; 7] = [Action::Cut, Action::Stitch, Action::Pin, Action::Reflatten, Action::Pack, Action::Straighten, Action::Even];

impl Action {
    pub fn icon(self) -> Icon {
        match self {
            Action::Cut => Icon::Scissors,
            Action::Stitch => Icon::Stitch,
            Action::Pin => Icon::Pin,
            Action::Unpin => Icon::Unpin,
            Action::Reflatten => Icon::Reflatten,
            Action::Pack => Icon::Pack,
            Action::Straighten => Icon::Straighten,
            Action::Even => Icon::EvenSize,
            Action::Unwrap => Icon::Unwrap,
            Action::FromFaces => Icon::Part,
            Action::CutFaces => Icon::CutFaces,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Action::Cut => "Cut",
            Action::Stitch => "Stitch",
            Action::Pin => "Pin",
            Action::Unpin => "Unpin",
            Action::Reflatten => "Re-flatten",
            Action::Pack => "Pack",
            Action::Straighten => "Straighten",
            Action::Even => "Even size",
            Action::Unwrap => "Unwrap again",
            Action::FromFaces => "Select their islands",
            Action::CutFaces => "Cut them out",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Action::Cut => "Part the layout along the selected edges: an island they divide comes apart, and each piece is laid out flat again",
            Action::Stitch => "Join the selected seams, or the seams between the selected islands: the smaller island is moved, turned and scaled onto the larger",
            Action::Pin => "Hold the selected vertices where they are when their island is laid out flat again",
            Action::Unpin => "Let the selected vertices go",
            Action::Reflatten => "Lay the selected islands out flat again, their cuts kept and their pins held",
            Action::Pack => "Fit the selected islands, or all of them, into the layout's square or between the guides, as large as they go",
            Action::Straighten => "Turn the selected islands, or all of them, so each runs square to the layout; with one edge selected, so that edge does",
            Action::Even => "Scale the selected islands, or all of them, so the texture is as fine on each",
            Action::Unwrap => "Lay the selected islands, or the whole layout, out flat again keeping the cuts it has",
            Action::FromFaces => "Select the islands the faces picked on the part are on",
            Action::CutFaces => "Cut the faces picked on the part out of the layout, along their border, as an island of their own",
        }
    }

    pub fn keys(self) -> &'static str {
        match self {
            Action::Cut => "C",
            Action::Stitch => "S",
            Action::Pin => "P",
            Action::Unpin => "Shift + P",
            Action::Pack => "Ctrl + P",
            _ => "",
        }
    }

    /// What it is called in a sentence saying it could not be done.
    fn verb(self) -> &'static str {
        match self {
            Action::Cut | Action::CutFaces => "cut",
            Action::Stitch => "stitch",
            Action::Pin | Action::Unpin => "pin",
            Action::Reflatten => "lay it out flat again",
            Action::Pack => "pack",
            Action::Straighten => "straighten",
            Action::Even => "even the sizes",
            Action::Unwrap => "unwrap again",
            Action::FromFaces => "select",
        }
    }
}

/// Where a pack puts the islands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackInto {
    Square,
    Guides,
}

/// What a pack's margin is given in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarginUnit {
    Millimetres,
    Pixels,
}

/// The Pack popover's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackSettings {
    pub margin: f32,
    pub unit: MarginUnit,
    /// The texture's side, for a margin in pixels.
    pub texture: u32,
    pub rotate: bool,
    /// Even the islands' sizes before packing.
    pub even: bool,
    pub into: PackInto,
}

impl Default for PackSettings {
    fn default() -> Self {
        PackSettings { margin: 8.0, unit: MarginUnit::Pixels, texture: 2048, rotate: true, even: false, into: PackInto::Square }
    }
}

impl PackSettings {
    /// The margin in the layout's units, on a layout of `density` metres of
    /// the part to a unit; a margin in millimetres needs one.
    pub fn margin_uv(&self, density: Option<f32>) -> Option<f32> {
        let uv = match self.unit {
            MarginUnit::Pixels => self.margin / self.texture.max(1) as f32,
            MarginUnit::Millimetres => self.margin / 1000.0 / density.filter(|d| *d > 0.0)?,
        };
        Some(uv.clamp(0.0, 0.25))
    }
}

/// The box between the guides: from the first to the last guide down the
/// layout, and across it; the square's sides where there are not two.
pub fn between_guides(guides: &[tequila_scene::guides::UvGuide]) -> Option<UvRect> {
    let span = |axis: GuideAxis| {
        let places: Vec<f32> = guides.iter().filter(|g| g.axis == axis).map(|g| g.at).collect();
        let low = places.iter().copied().fold(f32::INFINITY, f32::min);
        let high = places.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        (places.len() >= 2 && high - low > 1e-6).then_some((low, high))
    };
    let (across, down) = (span(GuideAxis::U), span(GuideAxis::V));
    if across.is_none() && down.is_none() {
        return None;
    }
    let (u0, u1) = across.unwrap_or((0.0, 1.0));
    let (v0, v1) = down.unwrap_or((0.0, 1.0));
    Some(UvRect { min: vec2(u0, v0), max: vec2(u1, v1) })
}

/// What the set picker does to the set shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetAction {
    Add,
    Copy,
    Delete,
    Up,
    Down,
}

impl SetAction {
    pub fn from_key(key: &str) -> Option<SetAction> {
        match key {
            "add" => Some(SetAction::Add),
            "copy" => Some(SetAction::Copy),
            "delete" => Some(SetAction::Delete),
            "up" => Some(SetAction::Up),
            "down" => Some(SetAction::Down),
            _ => None,
        }
    }
}

impl UvEditor {
    /// A layout command made at the end of the frame, with a word for the
    /// status bar if it is, or why not if it is refused.
    fn command(&self, edits: &mut Edits, command: Command, action: Action, done: String) {
        edits.layout.push(super::LayoutEdit { command, verb: action.verb(), done, carry: true });
    }

    /// The UV set shown, and the islands an action works on: those
    /// selected, or that the selected edges and vertices are on.
    pub(super) fn chosen_islands(&self, layout: &Layout) -> Vec<u32> {
        editing::islands_touched(layout, &self.selection)
    }

    /// Whether an action can be done now, or why not.
    pub(super) fn can(&self, action: Action, part: &Part, layout: Option<&Layout>) -> Result<(), &'static str> {
        let Some(layout) = layout else { return Err("Unwrap the part first") };
        let selection = &self.selection;
        match action {
            Action::Cut => {
                if editing::cuttable(layout, selection).is_empty() {
                    return Err("Select edges inside an island to cut along them (Edges, 3)");
                }
            }
            Action::Stitch => {
                if editing::stitch_edges(layout, selection).is_empty() {
                    return Err("Select seams, or islands that meet along seams, to stitch them");
                }
            }
            Action::Pin | Action::Unpin => {
                if editing::mesh_vertices(selection).is_empty() {
                    return Err("Select vertices to pin them (Vertices, 4)");
                }
            }
            Action::FromFaces | Action::CutFaces => {
                if part.faces.is_empty() {
                    return Err("Pick faces on the part in the viewport at Face level first");
                }
            }
            Action::Reflatten | Action::Pack | Action::Straighten | Action::Even | Action::Unwrap => {}
        }
        Ok(())
    }

    /// Do an island tool's action on the selection.
    pub(super) fn act(&mut self, action: Action, part: &Part, layout: &Layout, edits: &mut Edits) {
        if let Err(why) = self.can(action, part, Some(layout)) {
            self.say(why, true);
            return;
        }
        let (node, set) = (part.node, layout.uv.set);
        let plural = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
        match action {
            Action::Cut => {
                let edges = editing::cuttable(layout, &self.selection);
                let done = format!("Cut along {}", plural(edges.len(), "edge", "edges"));
                self.command(edits, Command::CutUvs { node, set, edges, reflatten: true }, action, done);
            }
            Action::Stitch => {
                let edges = editing::stitch_edges(layout, &self.selection);
                let done = format!("Stitched {}", plural(edges.len(), "seam edge", "seam edges"));
                self.command(edits, Command::StitchUvs { node, set, edges }, action, done);
            }
            Action::Pin | Action::Unpin => {
                let vertices = editing::mesh_vertices(&self.selection);
                let pins = self.pins.entry((node, set)).or_default();
                for v in &vertices {
                    if action == Action::Pin {
                        pins.insert(*v);
                    } else {
                        pins.remove(v);
                    }
                }
                let total = pins.len();
                let what = plural(vertices.len(), "vertex", "vertices");
                self.say(&if action == Action::Pin { format!("Pinned {what} · {total} in all") } else { format!("Unpinned {what} · {total} left") }, false);
            }
            Action::Reflatten => {
                let islands = self.islands_or_all(layout);
                let pins: Vec<u32> = self.pins_on(node, set, layout, &islands);
                let done = format!("Laid {} out flat again{}{}", plural(islands.len(), "island", "islands"), if pins.is_empty() { String::new() } else { format!(", {} held", plural(pins.len(), "pin", "pins")) }, if self.repack { ", and packed" } else { "" });
                self.command(edits, Command::ReflattenUvs { node, set, islands, pins, repack: self.repack }, action, done);
            }
            Action::Pack => {
                let Some(margin) = self.pack.margin_uv(layout.density()) else {
                    self.say("This layout is not one size all over, so a margin in millimetres means nothing: give it in pixels", true);
                    return;
                };
                let region = match self.pack.into {
                    PackInto::Square => UvRect::UNIT,
                    PackInto::Guides => match between_guides(&part.guides) {
                        Some(region) => region,
                        None => {
                            self.say("Drag out two guides across or down the layout to pack between them", true);
                            return;
                        }
                    },
                };
                let islands = self.chosen_islands(layout);
                let count = if islands.is_empty() { layout.uv.islands.len() } else { islands.len() };
                let options = PackOptions { margin, rotate: self.pack.rotate, normalize: self.pack.even, orient: false, region };
                let done = format!("Packed {} {}", plural(count, "island", "islands"), if self.pack.into == PackInto::Guides { "between the guides" } else { "into the square" });
                self.command(edits, Command::PackUvs { node, set, islands, options }, action, done);
            }
            Action::Straighten => {
                let edges = self.selection.edges();
                let (by, islands) = match edges.as_slice() {
                    [edge] if self.mode == Mode::Edges => (StraightenBy::Edge(*edge), Vec::new()),
                    _ => (StraightenBy::Axis, self.islands_or_all(layout)),
                };
                let done = match by {
                    StraightenBy::Edge(_) => "Turned the island so the edge runs square".to_string(),
                    StraightenBy::Axis => format!("Straightened {}", plural(islands.len(), "island", "islands")),
                };
                self.command(edits, Command::StraightenUvs { node, set, islands, by, to: Alignment::Nearest }, action, done);
            }
            Action::Even => {
                let islands = self.islands_or_all(layout);
                let done = format!("Evened the size of {}", plural(islands.len(), "island", "islands"));
                self.command(edits, Command::EvenUvDensity { node, set, islands, target: DensityTarget::Average }, action, done);
            }
            // As the Unwrap panel's options say: the shape tools do it.
            Action::Unwrap => self.shape.pending.push(super::shape_tools::Tool::Unwrap),
            Action::FromFaces => {
                let islands = layout.uv.islands_of_faces(&part.faces);
                self.mode = Mode::Islands;
                self.selection.set_all(islands.iter().filter_map(|&i| editing::island_item(layout, i)));
                self.island = islands.last().map(|&i| i as usize);
                self.say(&format!("Selected {} of the faces picked on the part", plural(islands.len(), "island", "islands")), false);
            }
            Action::CutFaces => {
                let edges = layout.uv.border_of_faces(&part.faces);
                if edges.is_empty() {
                    self.say("The faces picked are the whole of their islands already", true);
                    return;
                }
                // Selected by a face of theirs, which names the new island
                // once the cut has made it.
                self.mode = Mode::Islands;
                self.selection.set(Item::Island(part.faces[0]));
                let done = format!("Cut {} out along {}", plural(part.faces.len(), "face", "faces"), plural(edges.len(), "edge", "edges"));
                self.command(edits, Command::CutUvs { node, set, edges, reflatten: true }, action, done);
            }
        }
    }

    /// The selected islands, or every island when none is.
    pub(super) fn islands_or_all(&self, layout: &Layout) -> Vec<u32> {
        let chosen = self.chosen_islands(layout);
        if chosen.is_empty() { (0..layout.uv.islands.len() as u32).collect() } else { chosen }
    }

    /// The pins on some islands.
    fn pins_on(&self, node: tequila_scene::NodeId, set: usize, layout: &Layout, islands: &[u32]) -> Vec<u32> {
        self.pins.get(&(node, set)).map(|pins| pins.iter().copied().filter(|&v| layout.uv.island_of_vertex(v).is_some_and(|i| islands.contains(&i))).collect()).unwrap_or_default()
    }

    /// The layout's selection moved by `map`, as one `uv.transform`.
    pub(super) fn transform_layout(&mut self, part: &Part, layout: &Layout, map: UvAffine, edits: &mut Edits, done: &str) {
        let Some(selection) = editing::uv_selection(layout, &self.selection) else { return };
        if map == UvAffine::IDENTITY {
            return;
        }
        let command = Command::TransformUvs { node: part.node, set: layout.uv.set, selection, transform: map, pivot: tequila_scene::uv::Pivot::Origin };
        edits.layout.push(super::LayoutEdit { command, verb: "move it", done: done.to_string(), carry: true });
    }

    /// The frame round the layout's selection, as it is shown.
    pub(super) fn layout_frame(&self, layout: &Layout) -> Option<super::tools::Frame> {
        let moving = editing::moving(layout, &self.selection);
        editing::frame_of(layout, &moving, &self.panel_preview.unwrap_or(UvAffine::IDENTITY))
    }

    /// Turned a quarter, clockwise as the editor shows it, about the
    /// selection's middle.
    pub(super) fn quarter_layout(&mut self, part: &Part, layout: &Layout, turns: i32, edits: &mut Edits) {
        let Some(frame) = self.layout_frame(layout) else { return };
        let pivot = self.pivot_or(part, frame.centre);
        self.transform_layout(part, layout, UvAffine::quarter_turns(turns).about(pivot), edits, if turns > 0 { "Turned a quarter clockwise" } else { "Turned a quarter anticlockwise" });
    }

    /// Mirrored left to right (`across`) or top to bottom, about the
    /// selection's middle.
    pub(super) fn flip_layout(&mut self, part: &Part, layout: &Layout, across: bool, edits: &mut Edits) {
        let Some(frame) = self.layout_frame(layout) else { return };
        let map = if across { UvAffine::flip_u() } else { UvAffine::flip_v() };
        let pivot = self.pivot_or(part, frame.centre);
        self.transform_layout(part, layout, map.about(pivot), edits, if across { "Flipped horizontally" } else { "Flipped vertically" });
    }

    /// What the set picker does to the set shown.
    pub(super) fn set_action(&mut self, part: &Part, set: usize, action: SetAction, edits: &mut Edits) {
        let node = part.node;
        let count = part.sets;
        let (command, show, done) = match action {
            SetAction::Add => (Command::AddUvSet { node }, count, format!("Added UV set {}, unwrapped", count + 1)),
            SetAction::Copy => (Command::CopyUvSet { node, from: set, to: set + 1 }, set + 1, format!("Copied UV set {} as set {}", set + 1, set + 2)),
            SetAction::Delete => {
                if count <= 1 {
                    self.say("A part keeps one UV set at least", true);
                    return;
                }
                (Command::DeleteUvSet { node, set }, set.saturating_sub(1), format!("Deleted UV set {}", set + 1))
            }
            SetAction::Up => {
                if set == 0 {
                    return;
                }
                (Command::MoveUvSet { node, from: set, to: set - 1 }, set - 1, format!("UV set {} is now set {}", set + 1, set))
            }
            SetAction::Down => {
                if set + 1 >= count {
                    return;
                }
                (Command::MoveUvSet { node, from: set, to: set + 1 }, set + 1, format!("UV set {} is now set {}", set + 1, set + 2))
            }
        };
        edits.layout.push(super::LayoutEdit { command, verb: "change the sets", done, carry: false });
        edits.show_set = Some(show);
    }

    // ---- the Islands panel -----------------------------------------------------

    pub(super) fn islands_panel(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&Layout>, edits: &mut Edits) {
        let p = crate::theme::p();
        let set = self.shown_set;
        self.set_picker(ui, part, set, edits);
        let Some(layout) = layout else {
            ui.weak("Unwrap the part to lay its surface out in islands.");
            return;
        };
        ui.add_space(4.0);
        self.selection_summary(ui, part, layout);

        let mut act = None;
        let button = |ui: &mut egui::Ui, editor: &UvEditor, action: Action, act: &mut Option<Action>| {
            let ready = editor.can(action, part, Some(layout));
            ui.add_enabled_ui(ready.is_ok(), |ui| {
                let response = icons::labelled(ui, action.icon(), action.name(), false);
                let response = icons::tip(response, action.name(), match ready {
                    Ok(()) => action.detail(),
                    Err(why) => why,
                }, action.keys());
                if response.clicked() {
                    *act = Some(action);
                }
            });
        };

        crate::theme::section(ui, "Seams");
        ui.horizontal_wrapped(|ui| {
            button(ui, self, Action::Cut, &mut act);
            button(ui, self, Action::Stitch, &mut act);
            ui.label(egui::RichText::new(format!("{} seam edges", layout.seam_count())).size(12.0).color(p.text_dim));
        });

        crate::theme::section(ui, "Pins");
        ui.horizontal_wrapped(|ui| {
            button(ui, self, Action::Pin, &mut act);
            button(ui, self, Action::Unpin, &mut act);
            let count = self.pins.get(&(part.node, set)).map_or(0, |p| p.len());
            if ui.add_enabled(count > 0, egui::Button::new("Clear")).on_hover_text("Unpin every vertex of this layout").clicked() {
                self.pins.remove(&(part.node, set));
                self.say("Cleared the pins", false);
            }
            ui.label(egui::RichText::new(if count == 1 { "1 pin".to_string() } else { format!("{count} pins") }).size(12.0).color(p.text_dim));
        });
        ui.horizontal_wrapped(|ui| {
            button(ui, self, Action::Reflatten, &mut act);
            ui.add(crate::field::Checkbox::new(&mut self.repack, "Pack after")).on_hover_text("Pack the whole layout again once the islands are laid out");
        });

        crate::theme::section(ui, "Layout");
        ui.horizontal_wrapped(|ui| {
            let ready = self.can(Action::Pack, part, Some(layout)).is_ok();
            ui.add_enabled_ui(ready, |ui| {
                let response = icons::tip(icons::labelled(ui, Icon::Pack, "Pack", false), "Pack", Action::Pack.detail(), Action::Pack.keys());
                if response.clicked() {
                    act = Some(Action::Pack);
                }
            });
            let id = ui.id().with("uv-pack-options");
            let showing = ui.memory(|m| m.is_popup_open(id));
            let options = icons::button(ui, Icon::Sliders, showing, 30.0, egui::Sense::click());
            let options = icons::tip(options, "Pack options", "The margin between islands, whether they may turn, and where they go", "");
            // A click, or a script's asking, once.
            match (options.clicked(), self.pack_open.take()) {
                (true, _) => ui.memory_mut(|m| m.toggle_popup(id)),
                (false, Some(true)) => ui.memory_mut(|m| m.open_popup(id)),
                (false, Some(false)) if showing => ui.memory_mut(|m| m.close_popup()),
                _ => {}
            }
            egui::popup::popup_below_widget(ui, id, &options, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                ui.set_min_width(270.0);
                self.pack_options(ui, part, layout, &mut act);
            });
            button(ui, self, Action::Straighten, &mut act);
            button(ui, self, Action::Even, &mut act);
            button(ui, self, Action::Unwrap, &mut act);
        });

        crate::theme::section(ui, "Faces picked on the part");
        ui.horizontal_wrapped(|ui| {
            let count = part.faces.len();
            ui.label(egui::RichText::new(match count {
                0 => "None. Pick faces in the viewport at Face level.".to_string(),
                1 => "1 face".to_string(),
                n => format!("{n} faces"),
            }).size(12.0).color(p.text_dim));
        });
        ui.horizontal_wrapped(|ui| {
            button(ui, self, Action::FromFaces, &mut act);
            button(ui, self, Action::CutFaces, &mut act);
            self.project_faces_button(ui, part, layout);
        });

        crate::theme::section(ui, "Labels on an island");
        self.label_tools(ui, part, layout, edits);

        if let Some(action) = act {
            self.act(action, part, layout, edits);
        }
    }

    /// The Pack popover.
    fn pack_options(&mut self, ui: &mut egui::Ui, part: &Part, layout: &Layout, act: &mut Option<Action>) {
        let density = layout.density();
        crate::theme::section(ui, "Margin");
        ui.horizontal(|ui| {
            let suffix = if self.pack.unit == MarginUnit::Millimetres { " mm" } else { " px" };
            ui.add(egui::DragValue::new(&mut self.pack.margin).range(0.0..=500.0).speed(0.1).max_decimals(2).suffix(suffix)).on_hover_text("The gap between islands; half of it at the edges");
            let mut unit = if density.is_none() { MarginUnit::Pixels } else { self.pack.unit };
            ui.add_enabled_ui(density.is_some(), |ui| {
                ui.set_width(110.0);
                if crate::theme::segmented(ui, &mut unit, &[(MarginUnit::Millimetres, "mm"), (MarginUnit::Pixels, "px")]) {
                    self.pack.unit = unit;
                }
            })
            .response
            .on_disabled_hover_text("This layout is not one size all over, so the margin is in pixels");
        });
        if self.pack.unit == MarginUnit::Pixels || density.is_none() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("of a texture").color(crate::theme::dim()));
                ui.add(egui::DragValue::new(&mut self.pack.texture).range(64..=16384).speed(16.0).suffix(" px")).on_hover_text("The side of the texture the layout is for, which the margin is pixels of");
            });
        }
        crate::theme::section(ui, "Islands");
        ui.add(crate::field::Checkbox::new(&mut self.pack.rotate, "Turn islands to fit")).on_hover_text("An island may go in on its side, a quarter turn from how it was");
        ui.add(crate::field::Checkbox::new(&mut self.pack.even, "Even their sizes first")).on_hover_text("Scale them so the texture is as fine on each before packing");
        crate::theme::section(ui, "Into");
        let guides = between_guides(&part.guides);
        let mut into = self.pack.into;
        if crate::theme::segmented(ui, &mut into, &[(PackInto::Square, "The square"), (PackInto::Guides, "Between guides")]) {
            self.pack.into = into;
        }
        if self.pack.into == PackInto::Guides {
            ui.small(match guides {
                Some(region) => format!("{} × {} between the guides", self.scale.format(region.size().x), self.scale.format(region.size().y)),
                None => "Drag out two guides across or down the layout first.".to_string(),
            });
        }
        ui.add_space(6.0);
        if ui.button("Pack").clicked() {
            *act = Some(Action::Pack);
            ui.close_menu();
        }
    }

    /// The UV set picker, and what can be done to a set.
    fn set_picker(&mut self, ui: &mut egui::Ui, part: &Part, set: usize, edits: &mut Edits) {
        let p = crate::theme::p();
        ui.horizontal(|ui| {
            icons::paint(ui.painter(), egui::Rect::from_center_size(ui.cursor().min + egui::vec2(9.0, 12.0), egui::Vec2::splat(16.0)), Icon::UvSets, p.icon);
            ui.add_space(22.0);
            let shown = if part.sets == 0 { "No UV sets".to_string() } else { format!("UV set {} of {}       ", set + 1, part.sets) };
            let button = egui::Button::new(egui::RichText::new(shown).size(12.5)).fill(p.control).rounding(6.0).min_size(egui::vec2(140.0, 26.0));
            let response = ui.add_enabled(part.sets > 0, button).on_hover_text("The part's UV sets: the layout shown, and what reads each");
            let chevron = egui::Rect::from_center_size(response.rect.right_center() - egui::vec2(14.0, 0.0), egui::Vec2::splat(13.0));
            icons::paint(ui.painter(), chevron, Icon::ChevronDown, p.text_dim);
            let id = ui.id().with("uv-set-picker");
            match (response.clicked(), self.set_menu.take()) {
                (true, _) => ui.memory_mut(|m| m.toggle_popup(id)),
                (false, Some(true)) => ui.memory_mut(|m| m.open_popup(id)),
                (false, Some(false)) if ui.memory(|m| m.is_popup_open(id)) => ui.memory_mut(|m| m.close_popup()),
                _ => {}
            }
            let mut picked = None;
            egui::popup::popup_below_widget(ui, id, &response, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(220.0);
                for s in 0..part.sets {
                    let readers = part.readers(s);
                    let text = if readers.is_empty() { format!("Set {}", s + 1) } else { format!("Set {} · {readers}", s + 1) };
                    if ui.selectable_label(s == set, text).clicked() {
                        picked = Some(s);
                    }
                }
                ui.small("Labels and textures read their set by number, and keep to it as sets are added, moved and deleted.");
            });
            if let Some(s) = picked {
                self.show_set(s);
            }
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut tool = |ui: &mut egui::Ui, icon: Icon, enabled: bool, title: &str, detail: &str, action: SetAction| {
                ui.add_enabled_ui(enabled, |ui| {
                    let response = icons::tip(icons::button(ui, icon, false, 26.0, egui::Sense::click()), title, detail, "");
                    if response.clicked() {
                        self.set_action(part, set, action, edits);
                    }
                });
            };
            tool(ui, Icon::Plus, true, "Add a set", "A new UV set after the others, the part unwrapped into it", SetAction::Add);
            tool(ui, Icon::Copy, part.sets > 0, "Copy this set", "A copy of this layout, as the set after it", SetAction::Copy);
            tool(ui, Icon::ArrowUp, set > 0, "Move up", "Make this set come before the one above it. Labels and textures keep reading the layout they were on", SetAction::Up);
            tool(ui, Icon::ArrowDown, set + 1 < part.sets, "Move down", "Make this set come after the one below it. Labels and textures keep reading the layout they were on", SetAction::Down);
            tool(ui, Icon::Trash, part.sets > 1, "Delete this set", "Remove this layout. Labels and textures on it read the first set", SetAction::Delete);
        });
    }

    /// Show set `set`, letting go of what was selected on another.
    pub(super) fn show_set(&mut self, set: usize) {
        self.set_choice = set;
        // A label or texture selected says which set is shown; let it go,
        // so the choice is seen.
        self.selection.retain(|item| !matches!(item, Item::Label(_) | Item::Texture(_)));
    }

    /// What the selection is, measured: its size, its area on the part and
    /// the layout's stretch over it.
    fn selection_summary(&mut self, ui: &mut egui::Ui, part: &Part, layout: &Layout) {
        let p = crate::theme::p();
        let islands = editing::islands(layout, &self.selection);
        let line = |ui: &mut egui::Ui, text: String| {
            ui.label(egui::RichText::new(text).size(12.5).color(p.text));
        };
        if islands.is_empty() {
            let count = layout.uv.islands.len();
            ui.label(egui::RichText::new(match self.mode {
                Mode::Labels => format!("{count} islands. Click one on the canvas to fit labels to it, or pick Islands (2) to edit them."),
                Mode::Islands => format!("{count} islands. Click or box-select islands to work on them."),
                Mode::Edges => format!("{} seam edges. Select edges to cut along, or seams to stitch.", layout.seam_count()),
                Mode::Vertices => "Select vertices to move or pin them.".to_string(),
            }).size(12.0).color(p.text_dim));
            return;
        }
        let Some(frame) = self.layout_frame(layout) else { return };
        let distortion = self.distortion_for(part, layout);
        let measures = editing::measure(layout, &islands, distortion.as_ref().map(|d| d.as_slice()));
        line(ui, format!("{} · {} × {}", if islands.len() == 1 { format!("Island {} of {}", islands[0] + 1, layout.uv.islands.len()) } else { format!("{} islands", islands.len()) }, self.scale.format(frame.size.x), self.scale.format(frame.size.y)));
        let area = measures.surface * 1e6;
        let area = if area >= 100_000.0 { format!("{:.0} cm² on the part", area / 100.0) } else { format!("{area:.0} mm² on the part") };
        line(ui, format!("{area} · stretch {:.2}× area, {:.2} angle", measures.area, measures.angle));
        let mirrored = islands.iter().filter(|&&i| layout.uv.islands[i as usize].mirrored).count();
        if mirrored > 0 {
            ui.label(egui::RichText::new(if mirrored == 1 { "Mirrored: artwork reads backwards on it. Flip it to fix.".to_string() } else { format!("{mirrored} mirrored: artwork reads backwards on them. Flip to fix.") }).size(12.0).color(p.danger));
        }
    }

    /// The labels' tools, to put them on the island last clicked.
    fn label_tools(&mut self, ui: &mut egui::Ui, part: &Part, layout: &Layout, edits: &mut Edits) {
        let ready = self.island.is_some() && !self.selected_labels(super::canvas::labels_of(edits, part)).is_empty();
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(ready, |ui| {
                for (text, tip, action) in [
                    ("Fit to island", "As large as the island takes, keeping the label's shape", super::canvas::MenuAction::Fit),
                    ("Centre on island", "On the island's middle, as it is sized and turned", super::canvas::MenuAction::Centre),
                    ("Along the island", "Turned to run along the island's length, as Straighten turns the island", super::canvas::MenuAction::Along),
                ] {
                    if ui.button(text).on_hover_text(tip).on_disabled_hover_text("Select a label and click an island first").clicked() {
                        self.to_island(part, edits, layout, action);
                    }
                }
            });
            let any = !self.selected_labels(super::canvas::labels_of(edits, part)).is_empty();
            if ui.add_enabled(any, egui::Button::new("Square")).on_hover_text("Not turned").on_disabled_hover_text("Select a label first").clicked() {
                self.square_labels(part, edits);
            }
        });
    }

    /// The layout's stretch, triangle by triangle, as last worked out for
    /// this mesh and set (`UvEditor::keep_distortion`).
    pub(super) fn distortion_for(&self, part: &Part, layout: &Layout) -> Option<std::rc::Rc<Vec<tequila_scene::uv::Distortion>>> {
        let key = (part.mesh, layout.uv.set);
        self.distortion.as_ref().filter(|(k, _)| *k == key).map(|(_, d)| d.clone())
    }

    /// Work the stretch out again when the mesh or set it was for is not
    /// the one shown: only while the heatmap or a selection of islands
    /// wants it, as it costs a pass over the layout.
    pub(super) fn keep_distortion(&mut self, mesh: &tequila_scene::Mesh, key: tequila_scene::MeshKey, set: usize) {
        if self.distortion.as_ref().is_some_and(|(k, _)| *k == (key, set)) {
            return;
        }
        self.distortion = tequila_scene::uv::distortion(mesh, set).ok().map(|all| ((key, set), std::rc::Rc::new(all)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scene::guides::UvGuide;

    #[test]
    fn a_pack_s_margin_and_region_read_as_set() {
        let pixels = PackSettings { margin: 8.0, unit: MarginUnit::Pixels, texture: 2048, ..Default::default() };
        assert!((pixels.margin_uv(None).unwrap() - 8.0 / 2048.0).abs() < 1e-7);
        let mm = PackSettings { margin: 2.0, unit: MarginUnit::Millimetres, ..Default::default() };
        assert!((mm.margin_uv(Some(0.2)).unwrap() - 0.01).abs() < 1e-6, "2 mm of a 200 mm square");
        assert_eq!(mm.margin_uv(None), None, "no millimetres on a layout of no one size");
        let guides = [UvGuide::new(GuideAxis::U, 0.6), UvGuide::new(GuideAxis::U, 0.1), UvGuide::new(GuideAxis::V, 0.3)];
        let region = between_guides(&guides).unwrap();
        assert_eq!((region.min, region.max), (vec2(0.1, 0.0), vec2(0.6, 1.0)), "across between the two, down the whole square");
        assert_eq!(between_guides(&guides[2..]), None, "one guide makes no box");
        for (key, action) in [("add", SetAction::Add), ("delete", SetAction::Delete), ("down", SetAction::Down)] {
            assert_eq!(SetAction::from_key(key), Some(action));
        }
    }
}
