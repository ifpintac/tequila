//! The UV editor: a part's surface laid out flat (`tequila_scene::uv`), and
//! labels placed on it by hand - dragged, scaled and turned where they
//! print, typed to the millimetre, lined up with each other and with
//! guides, or put square on an island of the layout in one click. What a
//! planar projection cannot do, a label on an angled face or round a fillet,
//! is done here; and the material's textures that follow the layout are
//! placed here too. The layout itself is edited here as well: its islands
//! moved, turned and packed, cut apart and stitched together, laid out
//! flat again round pinned vertices.
//!
//! A window of its own (`crate::windows`), laid out as a drawing program
//! is: tools, what a click picks and the island tools along the top, rulers
//! along the canvas's top and left, the Layers, Transform, Align and
//! Islands panels down the right, and where the pointer is along the
//! bottom. Its pieces:
//!
//! - `view` and `canvas`: the layout on screen, drawn, picked and dragged;
//! - `tools`: what a press begins, and the drag while it lasts;
//! - `selection`: what is selected - labels and textures, or the layout's
//!   islands, edges and vertices - and the mode that says which a click
//!   picks; `editing`, the maths of picking and moving the layout's own;
//! - `actions`: the layout engine's edits, and the Islands panel;
//! - `snapping`: what any drag lands on, the same for all of them;
//! - `rulers` and `guides`; `units`, the millimetres they read in;
//! - `layers`, `transform` and `align`: the panels, their maths tested on
//!   their own; `placement`, a label's maths;
//! - `export`: the layout written as a picture; `link`: the editor and the
//!   viewport pointing at each other;
//! - `shortcuts`: the keys, live only while the editor is under the
//!   pointer or has the keyboard.
//!
//! Every edit is a command and one step to undo: a label's through
//! `material.set_decals`, a texture's through `material.set_mapping`, the
//! guides through `scene.set_uv_guides`, the layout's through the `uv.*`
//! commands - and a label on an island an edit moves goes with it, in the
//! same step.

mod actions;
mod align;
mod canvas;
mod editing;
mod export;
mod guides;
mod layers;
mod layout;
pub mod link;
mod placement;
mod prefs;
mod rulers;
mod selection;
mod shape_tools;
mod shortcuts;
mod snapping;
mod textures;
mod tools;
mod transform;
mod units;
#[cfg(test)]
mod tests;
mod view;

use crate::icons::{self, Icon};
use crate::project::Project;
use selection::{Item, Mode};
use std::collections::{BTreeSet, HashMap};
use tequila_core::math::{vec2, Vec2};
use tequila_materials::decal::Decal;
use tequila_materials::graph::TextureId;
use tequila_materials::material_type::Param;
use tequila_scene::guides::UvGuide;
use tequila_scene::uv::{Measure, UvAffine};
use tequila_scene::{Command, MaterialKey, MeshKey, NodeId, NodeKind};
use tequila_texture::projection::{Mapping, Projection};

/// Where the Labels tab leaves a request to open the editor on a label:
/// the material and the label's place, for the window to take next frame.
pub const OPEN: &str = "uv-editor-label";

/// How long a word in the status bar stays, in seconds.
const MESSAGE_SECONDS: f64 = 8.0;

/// Which part the editor shows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Subject {
    /// Whichever part is selected in the scene.
    Selected,
    /// This one, as a label's button or a script asked.
    Part(NodeId),
}

/// A script's hold on the pointer, so a picture can show a drag in
/// progress: where the pointer is, a drag held from one point of the
/// layout to another, and whether to let it go as a person would.
#[derive(Clone, Debug, Default)]
struct Scripted {
    pointer: Option<Vec2>,
    drag: Option<(Vec2, Vec2)>,
    release: bool,
    click: Option<Vec2>,
    /// What a script asked of the editor that needs the part and its
    /// layout at hand, done in the next frame.
    asks: Vec<Ask>,
    /// A face of the part held under the viewport's pointer.
    hover_face: Option<u32>,
}

/// A script's request carried out in the frame, as the buttons do it.
#[derive(Clone, Debug)]
enum Ask {
    Islands(Vec<u32>),
    Edges(Vec<[u32; 2]>),
    Vertices(Vec<u32>),
    SelectAll,
    Act(actions::Action),
    Transform(tequila_scripting::protocol::UvTransform),
    Set(actions::SetAction),
    Export(tequila_scripting::protocol::UvExport),
    Faces(Vec<u32>),
    Place(canvas::MenuAction),
    /// The selected labels printed by the layout rather than projected.
    OnLayout,
}

pub struct UvEditor {
    pub open: bool,
    /// Whether the pointer was over the editor last frame, drawn inside the
    /// main window: the viewport's keys step aside while it is.
    pub hovered: bool,
    /// The face under the viewport's pointer on the part shown, which the
    /// main window sets each frame for the layout to light it.
    pub viewport_hover: Option<link::ViewportHover>,
    subject: Subject,
    /// A label to open on, asked for this frame, for when the document is
    /// at hand.
    requested: Option<(MaterialKey, usize)>,
    view: view::View,
    tool: tools::Tool,
    /// What a click picks.
    mode: Mode,
    selection: selection::Selection,
    gesture: Option<tools::Gesture>,
    /// What the drag in progress can snap to, gathered as it began.
    targets: snapping::Targets,
    /// What it snapped to this frame.
    snap: snapping::Snap,
    /// The last measure, until the next or Esc.
    measure: Option<(Vec2, Vec2)>,
    prefs: prefs::Prefs,
    saved: prefs::Prefs,
    layout: Option<(MeshKey, usize, layout::Layout)>,
    /// The part and set the view was last framed for: a new mesh from an
    /// edit is the same layout moved, not one to frame again.
    framed_for: Option<(NodeId, usize)>,
    artwork: HashMap<TextureId, Option<egui::TextureHandle>>,
    island: Option<usize>,
    selected_guide: Option<usize>,
    typing: Option<guides::Typing>,
    layers: layers::State,
    transform: transform::Panel,
    align_to: align::Target,
    help: bool,
    help_anchor: Option<egui::Rect>,
    /// The pointer on the layout, for the rulers and the status bar.
    pointer: Option<Vec2>,
    /// What of the layout is under it.
    hover: editing::Hover,
    modifiers: egui::Modifiers,
    /// The unit everything reads in this frame.
    scale: units::Scale,
    canvas_rect: egui::Rect,
    frame_pending: bool,
    frame_request: Option<(Vec2, Vec2)>,
    zoom_request: Option<f32>,
    scripted: Scripted,
    cone: f32,
    /// A part opened from a label that has no layout, to unwrap before it
    /// is shown.
    unwrap_pending: bool,
    /// A script's align, carried out as the panel's buttons do it.
    scripted_align: Option<align::Align>,
    /// The layout's scene units per layout unit, where it has one size.
    density: Option<f32>,
    /// When the settings were last written, in the context's seconds.
    saved_at: f64,
    /// A change of the layout's selection typed or dragged into the
    /// Transform panel, shown until the field is let go.
    panel_preview: Option<UvAffine>,
    /// Vertices held still when an island is laid out flat again, by part
    /// and UV set, each by its mesh vertex: kept while the editor is open.
    pins: HashMap<(NodeId, usize), BTreeSet<u32>>,
    /// The UV set shown when no selected label or texture says which.
    set_choice: usize,
    /// The set shown this frame.
    shown_set: usize,
    /// The re-flatten's Pack after.
    repack: bool,
    pack: actions::PackSettings,
    /// The Pack popover, asked open or shut by a script.
    pack_open: Option<bool>,
    /// The set picker's list, asked open or shut by a script.
    set_menu: Option<bool>,
    /// The layout's stretch, triangle by triangle, for its mesh and set.
    distortion: Option<((MeshKey, usize), std::rc::Rc<Vec<tequila_scene::uv::Distortion>>)>,
    export: export::Dialog,
    /// The last word the editor had - what it did, or why it would not - and
    /// whether it was a refusal, and since when.
    message: Option<(String, bool, f64)>,
    /// What the viewport is to light on the part this frame.
    hint: link::Hint,
    /// The Unwrap, Shape and Symmetry panels' tools and their settings.
    shape: shape_tools::State,
}

impl Default for UvEditor {
    fn default() -> Self {
        let prefs = prefs::Prefs::load();
        UvEditor {
            open: false,
            hovered: false,
            viewport_hover: None,
            subject: Subject::Selected,
            requested: None,
            view: view::View::default(),
            tool: tools::Tool::Select,
            mode: Mode::Labels,
            selection: selection::Selection::default(),
            gesture: None,
            targets: snapping::Targets::new(),
            snap: snapping::Snap::none(),
            measure: None,
            prefs,
            saved: prefs,
            layout: None,
            framed_for: None,
            artwork: HashMap::new(),
            island: None,
            selected_guide: None,
            typing: None,
            layers: layers::State::default(),
            transform: transform::Panel::default(),
            align_to: align::Target::Selection,
            help: false,
            help_anchor: None,
            pointer: None,
            hover: editing::Hover::default(),
            modifiers: egui::Modifiers::NONE,
            scale: units::Scale::of(units::Unit::Layout, None),
            canvas_rect: egui::Rect::NOTHING,
            frame_pending: true,
            frame_request: None,
            zoom_request: None,
            scripted: Scripted::default(),
            cone: tequila_scene::uv::UnwrapOptions::default().cone_degrees,
            unwrap_pending: false,
            scripted_align: None,
            density: None,
            saved_at: f64::NEG_INFINITY,
            panel_preview: None,
            pins: HashMap::new(),
            set_choice: 0,
            shown_set: 0,
            repack: false,
            pack: actions::PackSettings::default(),
            pack_open: None,
            set_menu: None,
            distortion: None,
            export: export::Dialog::default(),
            message: None,
            hint: link::Hint::default(),
            shape: shape_tools::State::default(),
        }
    }
}

/// The part the editor works on this frame, read from the document.
pub(super) struct Part {
    pub node: NodeId,
    pub name: String,
    pub mesh: MeshKey,
    pub material: Option<MaterialKey>,
    pub labels: Vec<Decal>,
    pub textures: Vec<textures::Layer>,
    pub guides: Vec<UvGuide>,
    /// How many UV sets its mesh has.
    pub sets: usize,
    /// Its faces picked in the viewport, at Face level.
    pub faces: Vec<u32>,
}

impl Part {
    /// What reads UV set `set`: its labels and textures, counted.
    pub fn readers(&self, set: usize) -> String {
        let labels = self.labels.iter().filter(|l| l.mapping.projection == Projection::Uv && l.mapping.uv_set == set).count();
        let textures = self.textures.iter().filter(|t| t.mapping.uv_set == set).count();
        let mut out = Vec::new();
        match labels {
            0 => {}
            1 => out.push("1 label".to_string()),
            n => out.push(format!("{n} labels")),
        }
        match textures {
            0 => {}
            1 => out.push("1 texture".to_string()),
            n => out.push(format!("{n} textures")),
        }
        out.join(", ")
    }
}

/// An edit of the layout itself, made at the end of the frame: the
/// engine's command, what it is in a sentence saying it could not be done,
/// and what to say once it is.
pub(super) struct LayoutEdit {
    pub command: Command,
    pub verb: &'static str,
    pub done: String,
    /// Whether it moves islands, which the labels on them go with; a
    /// change to the sets renumbers the labels itself.
    pub carry: bool,
}

/// What this frame changed, made commands together at its end.
#[derive(Default)]
pub(super) struct Edits {
    /// The material's whole list of labels, as it is to be.
    pub labels: Option<Vec<Decal>>,
    pub textures: Vec<(Param, Mapping)>,
    pub guides: Option<Vec<UvGuide>>,
    /// Edits of the layout itself, in order.
    pub layout: Vec<LayoutEdit>,
    /// The UV set to show once they are made.
    pub show_set: Option<usize>,
    /// Faces of the part picked, as the viewport picks them at Face level.
    pub faces: Option<Vec<u32>>,
    pub unwrap: bool,
    pub add_label: bool,
    pub status: Option<String>,
}

impl Edits {
    pub fn labels_mut(&mut self, part: &Part) -> &mut Vec<Decal> {
        self.labels.get_or_insert_with(|| part.labels.clone())
    }
}

impl UvEditor {
    /// Open on a label, as the Labels tab's button asks: on its part, the
    /// label selected, and the part unwrapped if it has no layout yet.
    pub fn edit(&mut self, material: MaterialKey, label: usize) {
        self.open = true;
        self.requested = Some((material, label));
    }

    /// Open on whichever part is selected, as View > UV editor does.
    pub fn open_on_selection(&mut self) {
        self.open = true;
        self.subject = Subject::Selected;
        self.requested = None;
    }

    /// Open on a part.
    pub fn open_on(&mut self, node: NodeId) {
        self.open = true;
        self.subject = Subject::Part(node);
        self.selection.clear();
        self.frame_pending = true;
    }

    /// Let go of what belonged to a document that is no longer open.
    pub fn forget_document(&mut self) {
        self.subject = Subject::Selected;
        self.requested = None;
        self.selection.clear();
        self.gesture = None;
        self.layout = None;
        self.framed_for = None;
        self.artwork.clear();
        self.island = None;
        self.measure = None;
        self.typing = None;
        self.pins.clear();
        self.distortion = None;
        self.panel_preview = None;
        self.hint = link::Hint::default();
    }

    /// The part the viewport is to light, and what of it, while the editor
    /// is open.
    pub fn viewport_hint(&self) -> Option<&link::Hint> {
        (self.open && self.hint.node.is_some()).then_some(&self.hint)
    }

    /// The part the editor shows, if it is open on one.
    pub fn shown_part(&self) -> Option<NodeId> {
        if !self.open {
            return None;
        }
        self.hint.node.or(match self.subject {
            Subject::Part(node) => Some(node),
            Subject::Selected => None,
        })
    }

    /// A word in the status bar: what was done, or why not.
    pub(super) fn say(&mut self, text: &str, refusal: bool) {
        self.message = Some((text.to_string(), refusal, f64::NAN));
    }

    /// The part to show, by the subject.
    fn part(&self, project: &Project) -> Option<Part> {
        let document = project.document();
        let node = match self.subject {
            Subject::Part(node) => node,
            Subject::Selected => project.selection.active()?,
        };
        let scene_node = document.tree.get(node)?;
        let NodeKind::Part { mesh } = scene_node.kind else { return None };
        let material = scene_node.material;
        let definition = material.and_then(|key| document.materials.get(key));
        let labels = definition.map(|d| d.graph.decals.clone()).unwrap_or_default();
        let textures = definition
            .map(|d| textures::layers(&d.graph, |id| document.textures.get(id).map(|source| source.display_name())))
            .unwrap_or_default();
        let sets = document.geometry.get(mesh).map_or(0, |m| m.uv_sets.len());
        let faces = if project.selection.level() == tequila_scene::selection::SelectionLevel::Face { project.selection.faces(node).collect() } else { Vec::new() };
        Some(Part { node, name: scene_node.name.clone(), mesh, material, labels, textures, guides: scene_node.uv_guides.clone(), sets, faces })
    }

    /// The UV set to show: the first selected label's or texture's, or the
    /// one picked.
    fn uv_set(&self, part: &Part) -> usize {
        let chosen = self.selection.items().iter().find_map(|item| match item {
            Item::Label(i) => part.labels.get(*i).filter(|l| l.mapping.projection == Projection::Uv).map(|l| l.mapping.uv_set),
            Item::Texture(param) => part.textures.iter().find(|t| t.param == *param).map(|t| t.mapping.uv_set),
            _ => None,
        });
        chosen.unwrap_or(self.set_choice).min(part.sets.saturating_sub(1))
    }

    /// The next unit the rulers can read in.
    fn next_unit(&mut self, layout: &layout::Layout) -> Option<String> {
        if layout.density().is_none() {
            return Some("This layout has no one size, so it reads in UV units".into());
        }
        let at = units::Unit::ALL.iter().position(|u| *u == self.prefs.unit).unwrap_or(0);
        self.prefs.unit = units::Unit::ALL[(at + 1) % units::Unit::ALL.len()];
        None
    }

    /// Pick another kind of thing: what was selected of the old kind is let
    /// go, and a drag in progress ends.
    fn set_mode(&mut self, mode: Mode) {
        if self.mode != mode {
            self.mode = mode;
            self.selection.retain(|item| item.mode() == mode);
            self.gesture = None;
            self.panel_preview = None;
            self.snap = snapping::Snap::none();
        }
    }

    // ---- what the selection's labels can have done to them ------------------

    fn selected_labels(&self, labels: &[Decal]) -> Vec<usize> {
        self.selection.labels().into_iter().filter(|i| labels.get(*i).is_some_and(|l| !l.locked)).collect()
    }

    fn delete_selected(&mut self, part: &Part, edits: &mut Edits) {
        let mut doomed = self.selected_labels(canvas::labels_of(edits, part));
        if doomed.is_empty() {
            if let Some(index) = self.selected_guide.take() {
                let mut guides = edits.guides.clone().unwrap_or_else(|| part.guides.clone());
                if index < guides.len() {
                    guides.remove(index);
                    edits.guides = Some(guides);
                    edits.status = Some("Removed the guide".into());
                }
            }
            return;
        }
        doomed.sort_unstable();
        let labels = edits.labels_mut(part);
        for index in doomed.iter().rev() {
            labels.remove(*index);
        }
        self.selection.removed(&doomed);
        edits.status = Some(match doomed.len() {
            1 => "Deleted a label".into(),
            n => format!("Deleted {n} labels"),
        });
    }

    fn duplicate_selected(&mut self, part: &Part, edits: &mut Edits) {
        let chosen = self.selected_labels(canvas::labels_of(edits, part));
        if chosen.is_empty() {
            return;
        }
        // A little down and to the right, so the copy is seen to be one.
        let step = vec2(1.0, 1.0) * self.view_length(16.0);
        let labels = edits.labels_mut(part);
        let first = labels.len();
        for index in &chosen {
            let mut copy = labels[*index].clone();
            copy.name = format!("{} copy", copy.name);
            copy.mapping = placement::moved(copy.mapping, step);
            labels.push(copy);
        }
        self.selection.set_labels(first..first + chosen.len());
        edits.status = Some(match chosen.len() {
            1 => "Duplicated a label".into(),
            n => format!("Duplicated {n} labels"),
        });
    }

    /// `points` on screen, in layout units at the view's zoom.
    fn view_length(&self, points: f32) -> f32 {
        let canvas = if self.canvas_rect.is_positive() { self.canvas_rect } else { egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)) };
        self.view.xform(canvas).layout_length(points)
    }

    /// Every label that can be picked.
    fn select_all(&mut self, labels: &[Decal]) {
        self.selection.set_labels(labels.iter().enumerate().filter(|(_, l)| canvas::pickable(l)).map(|(i, _)| i));
    }

    /// Everything the mode picks.
    fn select_everything(&mut self, labels: &[Decal], layout: &layout::Layout) {
        match self.mode {
            Mode::Labels => self.select_all(labels),
            mode => self.selection.set_all(editing::all(layout, mode)),
        }
    }

    fn nudge(&mut self, part: &Part, layout: &layout::Layout, edits: &mut Edits, way: Vec2, big: bool) {
        let step = self.scale.nudge() * if big { 10.0 } else { 1.0 };
        let delta = way * step;
        if self.selection.is_layout() {
            self.transform_layout(part, layout, editing::moved(delta), edits, "Nudged");
            return;
        }
        if let Some(param) = self.selection.texture() {
            if let Some(layer) = part.textures.iter().find(|t| t.param == param) {
                let mapping = layer.mapping;
                edits.textures.push((param, placement::placed(mapping, vec2(0.5, 0.5), placement::centre(&mapping) + delta)));
            }
            return;
        }
        let chosen = self.selected_labels(canvas::labels_of(edits, part));
        if chosen.is_empty() {
            return;
        }
        let labels = edits.labels_mut(part);
        for index in chosen {
            labels[index].mapping = placement::moved(labels[index].mapping, delta);
        }
    }

    fn reorder(&mut self, part: &Part, edits: &mut Edits, from: usize, to: usize) {
        let labels = edits.labels_mut(part);
        if from >= labels.len() || to >= labels.len() || from == to {
            return;
        }
        let label = labels.remove(from);
        labels.insert(to, label);
        self.selection.reordered(from, to);
    }

    /// The selected labels to the top of the stack, or to the bottom, in
    /// their order.
    fn restack(&mut self, part: &Part, edits: &mut Edits, front: bool) {
        let chosen = self.selected_labels(canvas::labels_of(edits, part));
        if chosen.is_empty() {
            return;
        }
        let labels = edits.labels_mut(part);
        let (mut moving, mut rest) = (Vec::new(), Vec::new());
        for (i, label) in labels.drain(..).enumerate() {
            if chosen.contains(&i) {
                moving.push(label);
            } else {
                rest.push(label);
            }
        }
        let count = moving.len();
        if front {
            let start = rest.len();
            rest.extend(moving);
            *labels = rest;
            self.selection.set_labels(start..start + count);
        } else {
            moving.extend(rest);
            *labels = moving;
            self.selection.set_labels(0..count);
        }
    }

    fn flip(&mut self, part: &Part, layout: &layout::Layout, edits: &mut Edits, across: bool) {
        if self.selection.is_layout() {
            self.flip_layout(part, layout, across, edits);
            return;
        }
        let chosen = self.selected_labels(canvas::labels_of(edits, part));
        let mappings: Vec<Mapping> = chosen.iter().map(|i| canvas::labels_of(edits, part)[*i].mapping).collect();
        let Some(frame) = transform::frame_round(&mappings) else { return };
        let labels = edits.labels_mut(part);
        for index in &chosen {
            let m = labels[*index].mapping;
            labels[*index].mapping = if chosen.len() == 1 { placement::flipped(m, across) } else { placement::flipped_about(m, frame.centre, across) };
        }
    }

    /// The selection turned a quarter, clockwise as the editor shows it:
    /// the layout's own, or the selected labels each about its middle.
    fn quarter(&mut self, part: &Part, layout: &layout::Layout, edits: &mut Edits, turns: i32) {
        if self.selection.is_layout() {
            self.quarter_layout(part, layout, turns, edits);
            return;
        }
        let chosen = self.selected_labels(canvas::labels_of(edits, part));
        let labels = edits.labels_mut(part);
        for index in chosen {
            let m = labels[index].mapping;
            labels[index].mapping = placement::turned(m, placement::turn(&m) + turns as f32 * std::f32::consts::FRAC_PI_2);
        }
    }

    /// Fit, centre or turn each selected label to the island last clicked.
    fn to_island(&mut self, part: &Part, edits: &mut Edits, layout: &layout::Layout, how: canvas::MenuAction) {
        let Some(island) = self.island.filter(|i| *i < layout.points.len()) else { return };
        let chosen = self.selected_labels(canvas::labels_of(edits, part));
        let labels = edits.labels_mut(part);
        for index in chosen {
            let m = labels[index].mapping;
            if m.projection != Projection::Uv {
                continue;
            }
            labels[index].mapping = match how {
                canvas::MenuAction::Fit => placement::fitted(m, &layout.points[island]),
                canvas::MenuAction::Centre => placement::centred_on(m, &layout.points[island]),
                _ => placement::along(m, layout.axes[island]),
            };
        }
    }

    /// The selected labels, not turned.
    fn square_labels(&mut self, part: &Part, edits: &mut Edits) {
        let chosen = self.selected_labels(canvas::labels_of(edits, part));
        let labels = edits.labels_mut(part);
        for index in chosen {
            labels[index].mapping = placement::turned(labels[index].mapping, 0.0);
        }
    }

    fn align(&mut self, part: &Part, edits: &mut Edits, layout: &layout::Layout, how: align::Align) {
        if self.selection.is_layout() {
            self.align_islands(part, layout, edits, Some(how), None);
            return;
        }
        let chosen: Vec<usize> = self.selected_labels(canvas::labels_of(edits, part)).into_iter().filter(|i| canvas::labels_of(edits, part)[*i].mapping.projection == Projection::Uv).collect();
        let boxes: Vec<align::Box2> = chosen.iter().map(|i| placement::bounds(&canvas::labels_of(edits, part)[*i].mapping)).collect();
        let target = match self.align_to {
            align::Target::Selection if boxes.len() > 1 => align::union(&boxes),
            align::Target::Island => self.island.and_then(|i| layout.boxes.get(i).copied()),
            _ => Some((Vec2::ZERO, Vec2::splat(1.0))),
        };
        let Some(target) = target else {
            edits.status = Some("Click an island to align to it".into());
            return;
        };
        let moves = align::align(&boxes, target, how);
        let labels = edits.labels_mut(part);
        for (index, delta) in chosen.iter().zip(moves) {
            labels[*index].mapping = placement::moved(labels[*index].mapping, delta);
        }
    }

    fn distribute(&mut self, part: &Part, layout: &layout::Layout, edits: &mut Edits, across: bool) {
        if self.selection.is_layout() {
            self.align_islands(part, layout, edits, None, Some(across));
            return;
        }
        let chosen: Vec<usize> = self.selected_labels(canvas::labels_of(edits, part)).into_iter().filter(|i| canvas::labels_of(edits, part)[*i].mapping.projection == Projection::Uv).collect();
        let boxes: Vec<align::Box2> = chosen.iter().map(|i| placement::bounds(&canvas::labels_of(edits, part)[*i].mapping)).collect();
        let moves = align::distribute(&boxes, across);
        let labels = edits.labels_mut(part);
        for (index, delta) in chosen.iter().zip(moves) {
            labels[*index].mapping = placement::moved(labels[*index].mapping, delta);
        }
    }

    /// A projected label placed on the layout instead, filling the largest
    /// island.
    fn place_on_layout(&mut self, part: &Part, edits: &mut Edits, layout: &layout::Layout, index: usize) {
        let labels = edits.labels_mut(part);
        let Some(label) = labels.get_mut(index) else { return };
        label.mapping = on_layout(label.mapping, layout);
    }

    pub(super) fn menu_action(&mut self, action: canvas::MenuAction, part: &Part, layout: &layout::Layout, edits: &mut Edits) {
        use canvas::MenuAction;
        match action {
            MenuAction::Duplicate => self.duplicate_selected(part, edits),
            MenuAction::Delete => self.delete_selected(part, edits),
            MenuAction::ToFront => self.restack(part, edits, true),
            MenuAction::ToBack => self.restack(part, edits, false),
            MenuAction::Flip(across) => self.flip(part, layout, edits, across),
            MenuAction::Quarter(turns) => self.quarter(part, layout, edits, turns),
            MenuAction::Fit | MenuAction::Centre | MenuAction::Along => self.to_island(part, edits, layout, action),
            MenuAction::Square => self.square_labels(part, edits),
            MenuAction::Island(action) => self.act(action, part, layout, edits),
            MenuAction::LockGuides => self.prefs.guides_locked = !self.prefs.guides_locked,
            MenuAction::ClearGuides => {
                edits.guides = Some(Vec::new());
                self.selected_guide = None;
                edits.status = Some("Cleared the guides".into());
            }
            MenuAction::FrameSelection => self.frame_selection(canvas::labels_of(edits, part), part, layout),
            MenuAction::FrameAll => self.frame_all(canvas::labels_of(edits, part), part, layout),
            MenuAction::SelectAll => self.select_everything(canvas::labels_of(edits, part), layout),
            MenuAction::Shape(tool) => self.shape.pending.push(tool),
        }
    }

    fn shortcut(&mut self, action: shortcuts::Action, part: &Part, layout: &layout::Layout, edits: &mut Edits) {
        use shortcuts::Action;
        match action {
            Action::Tool(tool) => self.tool = tool,
            Action::Mode(mode) => self.set_mode(mode),
            Action::Nudge(way, big) => self.nudge(part, layout, edits, way, big),
            Action::Delete => self.delete_selected(part, edits),
            Action::Duplicate => self.duplicate_selected(part, edits),
            Action::SelectAll => self.select_everything(canvas::labels_of(edits, part), layout),
            Action::Escape => {
                if self.gesture.take().is_some() {
                    self.snap = snapping::Snap::none();
                } else if self.help {
                    self.help = false;
                } else if self.export.open {
                    self.export.open = false;
                } else if self.measure.take().is_none() {
                    self.selection.clear();
                    self.selected_guide = None;
                }
            }
            Action::FrameSelection => self.frame_selection(canvas::labels_of(edits, part), part, layout),
            Action::FrameAll => self.frame_all(canvas::labels_of(edits, part), part, layout),
            Action::Zoom(zoom) => self.zoom_request = Some(zoom),
            Action::Fit => self.frame_all(canvas::labels_of(edits, part), part, layout),
            Action::Grid => self.prefs.grid = !self.prefs.grid,
            Action::Guides => self.prefs.guides = !self.prefs.guides,
            Action::Snapping => self.prefs.snapping = !self.prefs.snapping,
            Action::Rulers => self.prefs.rulers = !self.prefs.rulers,
            Action::Help => self.help = !self.help,
            Action::Cut => self.act(actions::Action::Cut, part, layout, edits),
            Action::Stitch => self.act(actions::Action::Stitch, part, layout, edits),
            Action::Pin(on) => self.act(if on { actions::Action::Pin } else { actions::Action::Unpin }, part, layout, edits),
            Action::Pack => self.act(actions::Action::Pack, part, layout, edits),
            Action::Quarter(turns) => self.quarter(part, layout, edits, turns),
            Action::Flip(across) => self.flip(part, layout, edits, across),
            Action::Shape(tool) => self.shape.pending.push(tool),
        }
    }

    // ---- the panels ---------------------------------------------------------

    fn toolbar(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&layout::Layout>, edits: &mut Edits) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for info in &tools::TOOLS {
                let on = self.tool == info.tool;
                let response = icons::button(ui, info.icon, on, 30.0, egui::Sense::click());
                if icons::tip(response, info.name, info.detail, info.key).clicked() {
                    self.tool = info.tool;
                }
            }
            icons::vertical_divider(ui, 24.0);
            // What a click picks, then the layout engine's tools on it.
            for mode in Mode::ALL {
                let response = icons::button(ui, mode.icon(), self.mode == mode, 30.0, egui::Sense::click());
                if icons::tip(response, mode.name(), mode.detail(), mode.key_name()).clicked() {
                    self.set_mode(mode);
                }
            }
            icons::vertical_divider(ui, 24.0);
            let mut act = None;
            for action in actions::TOOLBAR {
                let ready = self.can(action, part, layout);
                ui.add_enabled_ui(ready.is_ok(), |ui| {
                    let response = icons::button(ui, action.icon(), false, 30.0, egui::Sense::click());
                    let detail = match ready {
                        Ok(()) => action.detail(),
                        Err(why) => why,
                    };
                    if icons::tip(response, action.name(), detail, action.keys()).clicked() {
                        act = Some(action);
                    }
                });
            }
            if let (Some(action), Some(layout)) = (act, layout) {
                self.act(action, part, layout, edits);
            }
            icons::vertical_divider(ui, 24.0);
            let toggle = |ui: &mut egui::Ui, value: &mut bool, icon: Icon, title: &str, detail: &str, keys: &str| {
                let response = icons::button(ui, icon, *value, 30.0, egui::Sense::click());
                if icons::tip(response, title, detail, keys).clicked() {
                    *value = !*value;
                }
            };
            toggle(ui, &mut self.prefs.snapping, Icon::Magnet, "Snapping", "Drags land on the grid, guides, the layout's edges and middle, islands and other labels. Hold Ctrl while dragging to drag freely.", "Ctrl + Shift + ;");
            toggle(ui, &mut self.prefs.grid, Icon::Grid, "Grid", "Lines across the layout at the spacing set in View options", "Ctrl + '");
            toggle(ui, &mut self.prefs.guides, Icon::Guides, "Guides", "Show guides. Drag one out of a ruler; drag it back to remove it.", "Ctrl + ;");
            toggle(ui, &mut self.prefs.rulers, Icon::Rulers, "Rulers", "Rulers along the top and left, in the part's millimetres", "Ctrl + R");
            toggle(ui, &mut self.prefs.artwork, Icon::Image, "Artwork", "Draw the labels' artwork, not just their outlines", "");
            toggle(ui, &mut self.prefs.edges, Icon::Edges, "Edges", "Draw every edge of the layout, faintly; seams and the part's edge are always drawn", "");
            let heat = icons::button(ui, Icon::Heat, self.prefs.stretch.is_some(), 30.0, egui::Sense::click());
            if icons::tip(heat, "Stretch", "A heatmap of how much the layout stretches the part, by area; by angle in View options", "").clicked() {
                self.prefs.stretch = if self.prefs.stretch.is_some() { None } else { Some(Measure::Area) };
            }
            ui.add_space(4.0);
            self.view_options(ui, part, edits);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let help = icons::button(ui, Icon::Keyboard, self.help, 30.0, egui::Sense::click());
                self.help_anchor = Some(help.rect);
                if icons::tip(help, "Shortcuts", "Every key the UV editor takes", "?").clicked() {
                    self.help = !self.help;
                }
                ui.add_enabled_ui(layout.is_some(), |ui| {
                    let export = icons::button(ui, Icon::ExportImage, self.export.open, 30.0, egui::Sense::click());
                    if icons::tip(export, "Export layout", "Save the layout as a picture to draw artwork over: an SVG, or a PNG", "").clicked() {
                        self.export.open = !self.export.open;
                    }
                });
                icons::vertical_divider(ui, 24.0);
                let unwrap = ui
                    .button("Unwrap")
                    .on_hover_text("Lay the part's surface out flat as its first UV set: its faces apart at sharp edges, curved surfaces in a few even pieces. The set it had moves up one.");
                ui.add(egui::DragValue::new(&mut self.cone).range(15.0..=85.0).suffix("°").speed(0.5))
                    .on_hover_text("How far a surface may turn within one piece of the layout: smaller makes more, flatter pieces");
                if unwrap.clicked() {
                    edits.unwrap = true;
                }
                if layout.is_none() {
                    ui.weak("No layout yet");
                }
                icons::vertical_divider(ui, 24.0);
                ui.add(egui::Label::new(egui::RichText::new(&part.name).strong()).truncate());
            });
        });
    }

    /// View options: the unit, the grid, the guides, the stretch heatmap,
    /// the artwork's opacity.
    fn view_options(&mut self, ui: &mut egui::Ui, part: &Part, edits: &mut Edits) {
        let density = self.density;
        let button = egui::Button::new(egui::RichText::new("View options").size(12.5)).fill(crate::theme::p().control).rounding(6.0);
        let response = ui.add(button).on_hover_text("The unit, the grid's spacing, the guides, the stretch heatmap and the artwork");
        let id = ui.id().with("uv-view-options");
        if response.clicked() {
            ui.memory_mut(|m| m.toggle_popup(id));
        }
        egui::popup::popup_below_widget(ui, id, &response, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            ui.set_min_width(260.0);
            crate::theme::section(ui, "Units");
            ui.add_enabled_ui(density.is_some(), |ui| {
                let mut unit = if density.is_some() { self.prefs.unit } else { units::Unit::Layout };
                if crate::theme::segmented(ui, &mut unit, &[(units::Unit::Millimetres, "mm"), (units::Unit::Centimetres, "cm"), (units::Unit::Inches, "in"), (units::Unit::Layout, "UV")]) {
                    self.prefs.unit = unit;
                }
            })
            .response
            .on_disabled_hover_text("This layout is not one size all over, so it reads in UV units: 1 across its square");
            crate::theme::section(ui, "Grid");
            let mut spacing = if density.is_some() { self.prefs.spacing } else { prefs::Spacing::Fraction };
            ui.add_enabled_ui(density.is_some(), |ui| {
                if crate::theme::segmented(ui, &mut spacing, &[(prefs::Spacing::Length, "Length"), (prefs::Spacing::Fraction, "Fraction of UV")]) {
                    self.prefs.spacing = spacing;
                }
            });
            ui.horizontal(|ui| {
                if spacing == prefs::Spacing::Length {
                    ui.add(egui::DragValue::new(&mut self.prefs.grid_mm).speed(0.1).range(0.01..=10_000.0).prefix("Every ").suffix(" mm").max_decimals(2))
                        .on_hover_text("The grid's spacing on the part");
                } else {
                    ui.add(egui::DragValue::new(&mut self.prefs.divisions).speed(0.2).range(1..=1000).prefix("Every 1/"))
                        .on_hover_text("The grid's spacing, as a fraction of the layout's square");
                }
                ui.add(egui::DragValue::new(&mut self.prefs.subdivisions).speed(0.1).range(1..=20).prefix("in ").suffix(" parts"))
                    .on_hover_text("Fainter lines between the grid's own");
            });
            crate::theme::section(ui, "Guides");
            ui.horizontal(|ui| {
                ui.add(crate::field::Checkbox::new(&mut self.prefs.guides_locked, "Lock guides")).on_hover_text("Guides stay where they are: they cannot be dragged or picked");
                if ui.add_enabled(!part.guides.is_empty(), egui::Button::new("Clear guides")).on_hover_text("Remove every guide on this part").clicked() {
                    edits.guides = Some(Vec::new());
                    self.selected_guide = None;
                    edits.status = Some("Cleared the guides".into());
                }
            });
            crate::theme::section(ui, "Stretch");
            let mut stretch = self.prefs.stretch;
            if crate::theme::segmented(ui, &mut stretch, &[(None, "Off"), (Some(Measure::Area), "Area"), (Some(Measure::Angle), "Angle")]) {
                self.prefs.stretch = stretch;
            }
            ui.small("A heatmap of how much the layout stretches the part: by area, blue where the texture is coarser and red where finer; by angle, yellow to red where it is squashed one way.");
            crate::theme::section(ui, "Artwork");
            let mut percent = self.prefs.artwork_opacity * 100.0;
            if ui.add(crate::field::Slider::new(&mut percent, 0.0..=100.0).text("Opacity").suffix("%").fixed_decimals(0)).on_hover_text("How strongly the labels' artwork is drawn over the layout").changed() {
                self.prefs.artwork_opacity = percent / 100.0;
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&layout::Layout>) {
        let p = crate::theme::p();
        let now = ui.input(|i| i.time);
        ui.horizontal(|ui| {
            let small = |text: String| egui::RichText::new(text).size(12.0).color(p.text_dim);
            match self.pointer {
                Some(at) => ui.label(small(format!("X {}   Y {}", self.scale.format(at.x), self.scale.format(at.y)))),
                None => ui.label(small("X —   Y —".into())),
            };
            icons::vertical_divider(ui, 16.0);
            match self.snap.note() {
                Some(note) => ui.label(egui::RichText::new(format!("Snapped to {note}")).size(12.0).color(crate::theme::hue(crate::theme::Hue::Pink))),
                None => ui.label(small(if self.prefs.snapping { "Snapping on".into() } else { "Snapping off".into() })),
            };
            icons::vertical_divider(ui, 16.0);
            ui.label(small(self.selection_words(part, layout)));
            if let (Some(island), Some(layout), Mode::Labels) = (self.island, layout, self.mode) {
                if let Some((low, high)) = layout.boxes.get(island) {
                    icons::vertical_divider(ui, 16.0);
                    ui.label(small(format!("Island {} of {} · {} × {}", island + 1, layout.boxes.len(), self.scale.format(high.x - low.x), self.scale.format(high.y - low.y))));
                }
            }
            // The last word: what was done, or why not.
            if let Some((text, refusal, since)) = &mut self.message {
                if since.is_nan() {
                    *since = now;
                }
                if now - *since < MESSAGE_SECONDS {
                    icons::vertical_divider(ui, 16.0);
                    let colour = if *refusal { p.danger } else { p.text };
                    // Cut short before the zoom's buttons, which it would
                    // run under.
                    let room = (ui.available_width() - 190.0).max(60.0);
                    ui.allocate_ui_with_layout(egui::vec2(room, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_max_width(room);
                        ui.add(egui::Label::new(egui::RichText::new(text.as_str()).size(12.0).color(colour)).truncate()).on_hover_text(text.as_str());
                    });
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(MESSAGE_SECONDS));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (text, zoom, tip) in [("200%", Some(2.0), "Twice as close (Ctrl + 2)"), ("100%", Some(1.0), "The layout's square fills the view (Ctrl + 1)"), ("Fit", None, "Everything in view (Home)")] {
                    if ui.add(egui::Button::new(egui::RichText::new(text).size(12.0)).frame(false)).on_hover_text(tip).clicked() {
                        match zoom {
                            Some(zoom) => self.zoom_request = Some(zoom),
                            None => self.frame_pending = true,
                        }
                    }
                }
                ui.label(small(format!("{:.0}%", self.view.zoom * 100.0)));
            });
        });
    }

    /// What is selected, in a few words, with its size for the layout's own.
    fn selection_words(&self, part: &Part, layout: Option<&layout::Layout>) -> String {
        let plural = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
        if let (true, Some(layout)) = (self.selection.is_layout(), layout) {
            let size = self.layout_frame(layout).map(|f| format!(" · {} × {}", self.scale.format(f.size.x), self.scale.format(f.size.y))).unwrap_or_default();
            return match self.mode {
                Mode::Islands => {
                    let islands = editing::islands(layout, &self.selection);
                    let surface: f32 = islands.iter().map(|&i| layout.uv.islands[i as usize].surface_area).sum::<f32>() * 1e6;
                    format!("{} selected{size} · {surface:.0} mm²", plural(islands.len(), "island", "islands"))
                }
                Mode::Edges => {
                    let edges = self.selection.edges();
                    let seams = edges.iter().filter(|e| layout.edge_named(**e).is_some_and(|i| layout.uv.edges[i as usize].kind == tequila_scene::uv::EdgeKind::Seam)).count();
                    format!("{} selected, {}{size}", plural(edges.len(), "edge", "edges"), plural(seams, "seam", "seams"))
                }
                _ => {
                    let vertices = self.selection.vertices();
                    let pinned = self.pins.get(&(part.node, layout.uv.set)).map_or(0, |pins| vertices.iter().filter(|v| pins.contains(v)).count());
                    format!("{} selected, {pinned} pinned{size}", plural(vertices.len(), "vertex", "vertices"))
                }
            };
        }
        let labels = self.selection.labels().len();
        match (labels, self.selection.texture()) {
            (_, Some(param)) => format!("{} selected", param.label()),
            (0, None) => match self.selected_guide {
                Some(_) => "A guide selected".into(),
                None => match (self.mode, layout) {
                    (Mode::Labels, _) | (_, None) => plural(part.labels.len(), "label", "labels"),
                    (_, Some(layout)) => format!("{} · {}", plural(layout.uv.islands.len(), "island", "islands"), plural(layout.seam_count(), "seam edge", "seam edges")),
                },
            },
            (1, None) => "1 label selected".into(),
            (n, None) => format!("{n} labels selected"),
        }
    }

    fn side_panel(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&layout::Layout>, project: &Project, edits: &mut Edits) {
        // ---- Layers -------------------------------------------------------
        crate::field::Fold::new(format!("Layers · {}", part.labels.len())).id_salt("uv-layers").default_open(true).show(ui, |ui| {
            let labels = canvas::labels_of(edits, part).to_vec();
            let ctx = ui.ctx().clone();
            let mut thumbnail = |id: TextureId| artwork(&mut self.artwork, project, id, &ctx);
            let actions = layers::show(ui, &mut self.layers, &labels, &part.textures, &self.selection, &mut thumbnail, part.material.is_some());
            for action in actions {
                match action {
                    layers::Action::Select(item, add) => {
                        // A label picked in its list is picked as a label.
                        self.set_mode(Mode::Labels);
                        if add {
                            self.selection.toggle(item);
                        } else {
                            self.selection.set(item);
                        }
                        self.selected_guide = None;
                    }
                    layers::Action::Visible(i, on) => {
                        if let Some(label) = edits.labels_mut(part).get_mut(i) {
                            label.enabled = on;
                        }
                        if !on {
                            self.selection.retain(|item| item != Item::Label(i));
                        }
                    }
                    layers::Action::Locked(i, on) => {
                        if let Some(label) = edits.labels_mut(part).get_mut(i) {
                            label.locked = on;
                        }
                        if on {
                            self.selection.retain(|item| item != Item::Label(i));
                        }
                    }
                    layers::Action::Rename(i, name) => {
                        if let Some(label) = edits.labels_mut(part).get_mut(i) {
                            label.name = name;
                        }
                    }
                    layers::Action::Opacity(i, opacity, _) => {
                        if let Some(label) = edits.labels_mut(part).get_mut(i) {
                            label.opacity = opacity;
                        }
                    }
                    layers::Action::Reorder { from, to } => self.reorder(part, edits, from, to),
                    layers::Action::Duplicate(i) => {
                        self.selection.set(Item::Label(i));
                        self.duplicate_selected(part, edits);
                    }
                    layers::Action::Delete(i) => {
                        self.selection.set(Item::Label(i));
                        self.delete_selected(part, edits);
                    }
                    layers::Action::Add => edits.add_label = true,
                }
            }
        });

        // ---- Transform ----------------------------------------------------
        crate::field::Fold::new("Transform").id_salt("uv-transform").default_open(true).show(ui, |ui| {
            if let (true, Some(layout)) = (self.selection.is_layout(), layout) {
                let Some(frame) = self.layout_frame(layout) else { return };
                let what = match self.mode {
                    Mode::Islands => "The selected islands, by the box round them",
                    Mode::Edges => "The selected edges' ends, by the box round them",
                    _ => "The selected vertices, by the box round them",
                };
                ui.label(egui::RichText::new(what).size(12.0).color(crate::theme::dim()));
                match transform::layout(ui, &mut self.transform, &frame, self.scale) {
                    Some(transform::LayoutChange::Map(map)) => {
                        self.panel_preview = Some(self.panel_preview.unwrap_or(UvAffine::IDENTITY).then(&map));
                    }
                    Some(transform::LayoutChange::Flip(across)) => self.flip_layout(part, layout, across, edits),
                    Some(transform::LayoutChange::Quarter(turns)) => self.quarter_layout(part, layout, turns, edits),
                    None => {}
                }
                return;
            }
            if let Some(param) = self.selection.texture() {
                if let Some(layer) = part.textures.iter().find(|t| t.param == param) {
                    ui.label(egui::RichText::new(format!("{} · {}", param.label(), layer.name)).color(crate::theme::dim()));
                    // As it is shown, while a drag moves it.
                    let shown = canvas::shown_mapping(self.gesture.as_ref(), Item::Texture(param), &part.labels, part).unwrap_or(layer.mapping);
                    if let Some(mapping) = transform::texture(ui, &shown) {
                        edits.textures.push((param, mapping));
                    }
                }
                return;
            }
            let chosen = self.selected_labels(canvas::labels_of(edits, part));
            let projected: Vec<usize> = chosen.iter().copied().filter(|i| canvas::labels_of(edits, part)[*i].mapping.projection != Projection::Uv).collect();
            if let (false, Some(layout)) = (projected.is_empty(), layout) {
                ui.weak(if projected.len() == 1 { "This label is projected flat, not placed by the UV layout." } else { "These labels are projected flat, not placed by the UV layout." });
                if ui.button("Place it on the UV layout").on_hover_text("Print the label by the part's UV layout instead of projecting it, so it can sit on any face").clicked() {
                    for index in &projected {
                        self.place_on_layout(part, edits, layout, *index);
                    }
                }
                return;
            }
            if chosen.is_empty() && self.mode.is_layout() {
                ui.weak("Select islands, edges or vertices to place them by number.");
                return;
            }
            let mut labels = canvas::labels_of(edits, part).to_vec();
            // The numbers follow a drag as it goes.
            if let Some(tools::Gesture::Transform { current, .. }) = &self.gesture {
                for (item, mapping) in current {
                    if let Item::Label(i) = item {
                        if let Some(label) = labels.get_mut(*i) {
                            label.mapping = *mapping;
                        }
                    }
                }
            }
            if transform::labels(ui, &mut self.transform, &mut labels, &chosen, self.scale) {
                edits.labels = Some(labels);
            }
        });

        // ---- Align --------------------------------------------------------
        crate::field::Fold::new("Align").id_salt("uv-align").default_open(true).show(ui, |ui| {
            let targets = [align::Target::Selection, align::Target::Island, align::Target::Layout].map(|t| (t, t.label()));
            crate::theme::segmented(ui, &mut self.align_to, &targets);
            ui.add_space(4.0);
            let (count, things) = match (self.selection.is_layout(), layout) {
                (true, Some(layout)) => (editing::islands(layout, &self.selection).len(), "islands"),
                _ => (self.selected_labels(canvas::labels_of(edits, part)).len(), "labels"),
            };
            let can_align = count >= 1;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for how in align::Align::ALL {
                    ui.add_enabled_ui(can_align, |ui| {
                        let response = icons::button(ui, how.icon(), false, 28.0, egui::Sense::click());
                        let detail = match self.align_to {
                            align::Target::Selection => "To the box round the selection; one alone, to the layout's square",
                            align::Target::Island => "To the island last clicked",
                            align::Target::Layout => "To the layout's square",
                        };
                        let detail = if can_align { detail.to_string() } else { format!("Select {things} to line them up") };
                        if icons::tip(response, how.name(), &detail, "").clicked() {
                            if let Some(layout) = layout {
                                self.align(part, edits, layout, how);
                            }
                        }
                    });
                }
                icons::vertical_divider(ui, 22.0);
                for (across, icon, name) in [(true, Icon::DistributeAcross, "Spread across"), (false, Icon::DistributeDown, "Spread down")] {
                    ui.add_enabled_ui(count >= 3, |ui| {
                        let response = icons::button(ui, icon, false, 28.0, egui::Sense::click());
                        let detail = format!("Equal gaps between three or more {things}, the first and last staying put");
                        if icons::tip(response, name, &detail, "").clicked() {
                            if let Some(layout) = layout {
                                self.distribute(part, layout, edits, across);
                            }
                        }
                    });
                }
            });
        });

        // ---- Islands ------------------------------------------------------
        crate::field::Fold::new("Islands").id_salt("uv-islands").default_open(true).show(ui, |ui| {
            self.islands_panel(ui, part, layout, edits);
        });

        // ---- Unwrap, Shape, Symmetry -----------------------------------------
        self.shape_panels(ui, part, layout, project);
    }

    /// The help popover: every key, by group, as key caps.
    fn help_popover(&mut self, ctx: &egui::Context) {
        let Some(anchor) = self.help_anchor else { return };
        let p = crate::theme::p();
        let area = egui::Area::new(egui::Id::new("uv-editor-help"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(anchor.right_bottom() + egui::vec2(0.0, 6.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(p.raised).inner_margin(egui::Margin::same(14.0)).show(ui, |ui| {
                    ui.label(egui::RichText::new("UV editor shortcuts").font(crate::theme::strong(14.0)));
                    ui.small("Active while the editor is under the pointer or has the keyboard.");
                    // Columns of groups, each row the keys in a column of
                    // their own and what they do wrapped beside them.
                    let (column, keys_width) = (300.0, 118.0);
                    // Rows as tight as the key caps, so the whole list fits beside
                    // the canvas without scrolling.
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.spacing_mut().interact_size.y = 20.0;
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 18.0;
                        let starts = shortcuts::COLUMNS;
                        for (n, &start) in starts.iter().enumerate() {
                            let end = starts.get(n + 1).copied().unwrap_or(shortcuts::LIST.len());
                            ui.allocate_ui_with_layout(egui::vec2(column, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.set_width(column);
                                for (title, keys) in &shortcuts::LIST[start..end] {
                                    crate::theme::section(ui, title);
                                    for (binding, what) in keys.iter() {
                                        ui.horizontal(|ui| {
                                            ui.allocate_ui_with_layout(egui::vec2(keys_width, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                                ui.set_width(keys_width);
                                                crate::theme::key_caps(ui, binding);
                                            });
                                            ui.add(egui::Label::new(egui::RichText::new(*what).size(12.0).color(p.text_dim)).wrap());
                                        });
                                    }
                                }
                            });
                        }
                    });
                });
            });
        // A click anywhere else puts it away.
        let clicked_outside = ctx.input(|i| i.pointer.any_click()) && ctx.pointer_interact_pos().is_some_and(|at| !area.response.rect.contains(at) && !anchor.contains(at));
        if clicked_outside {
            self.help = false;
        }
    }

    /// Drive the editor from a script: open it on a part or a label, set
    /// its tool, mode and zoom, select labels, a texture or the layout's
    /// own, do what the island tools do, show its shortcuts, and hold a drag
    /// where a screenshot can show it.
    pub fn script(&mut self, change: &tequila_scripting::protocol::UvEditorChange, project: &Project) -> Result<(), String> {
        // Checked first, so a request with one bad field changes nothing.
        let mode = match &change.mode {
            Some(key) => Some(Mode::from_key(key).ok_or_else(|| format!("`{key}` is not a mode: labels, islands, edges or vertices"))?),
            None => None,
        };
        let heatmap = match change.heatmap.as_deref() {
            None => None,
            Some("area") => Some(Some(Measure::Area)),
            Some("angle") => Some(Some(Measure::Angle)),
            Some("off") | Some("none") => Some(None),
            Some(other) => return Err(format!("`{other}` is not a heatmap: area, angle or off")),
        };
        let set_action = match &change.set_action {
            Some(key) => Some(actions::SetAction::from_key(key).ok_or_else(|| format!("`{key}` is not a set action: add, copy, delete, up or down"))?),
            None => None,
        };
        let place = match change.place.as_deref() {
            None => None,
            Some("fit") => Some(Ask::Place(canvas::MenuAction::Fit)),
            Some("centre") | Some("center") => Some(Ask::Place(canvas::MenuAction::Centre)),
            Some("along") => Some(Ask::Place(canvas::MenuAction::Along)),
            Some("square") => Some(Ask::Place(canvas::MenuAction::Square)),
            Some("layout") => Some(Ask::OnLayout),
            Some(other) => return Err(format!("`{other}` is not a way to place a label: layout, fit, centre, along or square")),
        };
        self.shape_script(&change.shape)?;
        if let Some(pack) = &change.pack {
            if let Some(unit) = &pack.unit {
                self.pack.unit = match unit.as_str() {
                    "mm" => actions::MarginUnit::Millimetres,
                    "px" => actions::MarginUnit::Pixels,
                    other => return Err(format!("`{other}` is not a margin's unit: mm or px")),
                };
            }
            if let Some(into) = &pack.into {
                self.pack.into = match into.as_str() {
                    "square" => actions::PackInto::Square,
                    "guides" => actions::PackInto::Guides,
                    other => return Err(format!("`{other}` is not where to pack: square or guides")),
                };
            }
            if let Some(margin) = pack.margin {
                self.pack.margin = margin.clamp(0.0, 500.0);
            }
            if let Some(texture) = pack.texture {
                self.pack.texture = texture.clamp(64, 16384);
            }
            if let Some(rotate) = pack.rotate {
                self.pack.rotate = rotate;
            }
        }
        if let Some(export) = &change.export {
            self.export.take(export)?;
        }
        if let Some(node) = change.part {
            let id = NodeId(node);
            match project.document().tree.get(id).map(|n| &n.kind) {
                Some(NodeKind::Part { .. }) => self.open_on(id),
                Some(_) => return Err(format!("{id} is not a part, so it has no UV layout")),
                None => return Err(format!("there is no node {node}")),
            }
        }
        if let Some((material, index)) = change.label {
            let key = MaterialKey(material);
            let Some(definition) = project.document().materials.get(key) else {
                return Err(format!("there is no material {material}"));
            };
            if index >= definition.graph.decals.len() {
                return Err(format!("material {material} has {} labels; there is no label {index}", definition.graph.decals.len()));
            }
            self.edit(key, index);
        }
        if let Some(tool) = &change.tool {
            self.tool = tools::Tool::from_key(tool).ok_or_else(|| format!("`{tool}` is not a tool: select, move, rotate, scale or measure"))?;
        }
        if let Some(mode) = mode {
            self.set_mode(mode);
        }
        if let Some(stretch) = heatmap {
            self.prefs.stretch = stretch;
        }
        if let Some(set) = change.set {
            self.show_set(set);
        }
        if change.fit {
            self.frame_pending = true;
        }
        if let Some(zoom) = change.zoom {
            self.zoom_request = Some(zoom.clamp(view::MIN_ZOOM, view::MAX_ZOOM));
        }
        if let Some(labels) = &change.select {
            self.set_mode(Mode::Labels);
            self.selection.set_labels(labels.iter().copied());
        }
        if let Some(texture) = &change.texture {
            let param = Param::ALL.iter().copied().find(|p| p.key() == texture.as_str()).ok_or_else(|| format!("`{texture}` is not a parameter"))?;
            self.set_mode(Mode::Labels);
            self.selection.set(Item::Texture(param));
        }
        if let Some(island) = change.island {
            self.island = Some(island);
        }
        let asks = &mut self.scripted.asks;
        if let Some(islands) = &change.islands {
            asks.push(Ask::Islands(islands.iter().map(|&i| i as u32).collect()));
        }
        if let Some(edges) = &change.edges {
            asks.push(Ask::Edges(edges.iter().map(|[a, b]| [*a as u32, *b as u32]).collect()));
        }
        if let Some(vertices) = &change.vertices {
            asks.push(Ask::Vertices(vertices.iter().map(|&v| v as u32).collect()));
        }
        if change.select_all {
            asks.push(Ask::SelectAll);
        }
        if let Some(faces) = &change.faces {
            asks.push(Ask::Faces(faces.iter().map(|&f| f as u32).collect()));
        }
        if let Some(repack) = change.repack {
            self.repack = repack;
        }
        let asks = &mut self.scripted.asks;
        for (wanted, action) in [
            (change.from_faces, actions::Action::FromFaces),
            (change.cut, actions::Action::Cut),
            (change.stitch, actions::Action::Stitch),
            (change.pin == Some(true), actions::Action::Pin),
            (change.pin == Some(false), actions::Action::Unpin),
            (change.reflatten, actions::Action::Reflatten),
            (change.pack.is_some(), actions::Action::Pack),
            (change.straighten, actions::Action::Straighten),
            (change.even, actions::Action::Even),
            (change.unwrap, actions::Action::Unwrap),
            (change.cut_faces, actions::Action::CutFaces),
        ] {
            if wanted {
                asks.push(Ask::Act(action));
            }
        }
        if let Some(transform) = &change.transform {
            asks.push(Ask::Transform(transform.clone()));
        }
        if let Some(action) = set_action {
            asks.push(Ask::Set(action));
        }
        if let Some(export) = &change.export {
            asks.push(Ask::Export(export.clone()));
        }
        if let Some(place) = place {
            asks.push(place);
        }
        if let Some(open) = change.pack_options {
            self.pack_open = Some(open);
        }
        if let Some(open) = change.set_menu {
            self.set_menu = Some(open);
        }
        if let Some(open) = change.export_dialog {
            self.export.open = open;
        }
        if let Some(face) = change.hover_face {
            self.scripted.hover_face = face.map(|f| f as u32);
        }
        if let Some(help) = change.help {
            self.help = help;
        }
        if let Some(pointer) = change.pointer {
            self.scripted.pointer = pointer.map(|[u, v]| vec2(u, v));
        }
        match change.drag {
            Some(Some([[u0, v0], [u1, v1]])) => {
                self.gesture = None;
                self.scripted.drag = Some((vec2(u0, v0), vec2(u1, v1)));
            }
            Some(None) => {
                self.scripted.drag = None;
                self.gesture = None;
                self.snap = snapping::Snap::none();
            }
            None => {}
        }
        if change.release {
            self.scripted.release = true;
        }
        if let Some([u, v]) = change.click {
            self.scripted.click = Some(vec2(u, v));
        }
        if let Some([[u0, v0], [u1, v1]]) = change.frame {
            self.frame_request = Some(layout::rect(vec2(u0, v0), vec2(u1, v1)));
        }
        if let Some(how) = &change.align {
            self.scripted_align = Some(align::Align::from_key(how).ok_or_else(|| format!("`{how}` is not a way to align: left, centre, right, top, middle or bottom"))?);
        }
        self.open = true;
        Ok(())
    }

    /// A script's requests that need the part and its layout, done as the
    /// buttons and keys do them.
    fn scripted_asks(&mut self, part: &Part, layout: Option<&layout::Layout>, project: &Project, edits: &mut Edits) {
        for ask in std::mem::take(&mut self.scripted.asks) {
            let Some(layout) = layout else {
                self.say("The part has no layout to work on: unwrap it first", true);
                continue;
            };
            match ask {
                Ask::Islands(islands) => {
                    self.set_mode(Mode::Islands);
                    self.selection.set_all(islands.iter().filter_map(|&i| editing::island_item(layout, i)));
                    self.island = islands.last().map(|&i| i as usize).filter(|&i| i < layout.uv.islands.len());
                }
                Ask::Edges(edges) => {
                    self.set_mode(Mode::Edges);
                    self.selection.set_all(edges.into_iter().filter(|e| layout.edge_named(*e).is_some()).map(Item::Edge));
                }
                Ask::Vertices(vertices) => {
                    self.set_mode(Mode::Vertices);
                    self.selection.set_all(vertices.into_iter().filter(|v| layout.place_of(*v).is_some()).map(Item::Vertex));
                }
                Ask::SelectAll => self.select_everything(canvas::labels_of(edits, part), layout),
                Ask::Act(action) => self.act(action, part, layout, edits),
                Ask::Transform(transform) => self.scripted_transform(part, layout, &transform, edits),
                Ask::Set(action) => self.set_action(part, self.shown_set, action, edits),
                Ask::Export(request) => {
                    let path = std::path::PathBuf::from(&request.path);
                    self.write_export(project, part, layout.uv.set, &path);
                }
                Ask::Faces(faces) => edits.faces = Some(faces),
                Ask::Place(canvas::MenuAction::Square) => self.square_labels(part, edits),
                Ask::Place(how) => self.to_island(part, edits, layout, how),
                Ask::OnLayout => {
                    for index in self.selected_labels(canvas::labels_of(edits, part)) {
                        if canvas::labels_of(edits, part)[index].mapping.projection != Projection::Uv {
                            self.place_on_layout(part, edits, layout, index);
                        }
                    }
                }
            }
        }
    }

    /// A script's move, turn, scale or flip, about the Transform panel's
    /// chosen point, of the layout's selection or the labels'.
    fn scripted_transform(&mut self, part: &Part, layout: &layout::Layout, change: &tequila_scripting::protocol::UvTransform, edits: &mut Edits) {
        if let Some(turns) = change.quarter {
            self.quarter(part, layout, edits, turns);
        }
        if let Some(flip) = &change.flip {
            self.flip(part, layout, edits, flip != "vertical");
        }
        if change.offset.is_none() && change.turn.is_none() && change.scale.is_none() {
            return;
        }
        if self.selection.is_layout() {
            let Some(frame) = self.layout_frame(layout) else { return };
            let pivot = self.transform.reference.point(&frame);
            let mut map = UvAffine::IDENTITY;
            if let Some([sx, sy]) = change.scale {
                map = map.then(&editing::scaled(pivot, vec2(sx, sy)));
            }
            if let Some(degrees) = change.turn {
                // Anticlockwise as the panel counts, which on the layout,
                // v down, is a negative turn.
                map = map.then(&editing::turned(pivot, -degrees.to_radians()));
            }
            if let Some([du, dv]) = change.offset {
                map = map.then(&editing::moved(vec2(du, dv)));
            }
            self.transform_layout(part, layout, map, edits, "Moved");
            return;
        }
        let chosen = self.selected_labels(canvas::labels_of(edits, part));
        let labels = edits.labels_mut(part);
        for index in chosen {
            let mut m = labels[index].mapping;
            if let Some([sx, sy]) = change.scale {
                m = placement::resized(m, placement::size(&m) * vec2(sx, sy));
            }
            if let Some(degrees) = change.turn {
                m = placement::turned(m, placement::turn(&m) - degrees.to_radians());
            }
            if let Some([du, dv]) = change.offset {
                m = placement::moved(m, vec2(du, dv));
            }
            labels[index].mapping = m;
        }
    }

    /// The layout written as the Export dialog says, to `path`.
    fn write_export(&mut self, project: &Project, part: &Part, set: usize, path: &std::path::Path) {
        let Some(mesh) = project.document().geometry.get(part.mesh) else { return };
        let mut picture = self.export.picture();
        if let Some(format) = export::Format::of_path(path) {
            self.export.format = format;
        }
        picture.size = picture.size.clamp(16, tequila_scene::uv::export::MOST_PIXELS);
        match export::write(mesh, set, &picture, path) {
            Ok(bytes) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
                self.say(&format!("Wrote the layout to {name} ({} kB)", bytes.div_ceil(1024)), false);
                self.export.open = false;
            }
            Err(error) => self.say(&error, true),
        }
    }

    /// The editor's state, for the scripting channel's `ui` query.
    pub fn state(&self) -> tequila_format::json::Value {
        use tequila_format::json::Value;
        let mut out = tequila_format::json_write::Object::new();
        let numbers = |list: Vec<u32>| Value::Array(list.into_iter().map(|n| Value::Number(n as f64)).collect());
        out.set("open", self.open)
            .set("tool", self.tool.key())
            .set("mode", self.mode.key())
            .set("zoom", self.view.zoom)
            .set("unit", self.scale.unit.suffix())
            .set("selected", self.selection.len() as f64)
            .set("labels", Value::Array(self.selection.labels().into_iter().map(|i| Value::Number(i as f64)).collect()))
            .set("help", self.help)
            .set("snapped", self.snap.note().map(|n| Value::String(n.into())).unwrap_or(Value::Null))
            .set("measure", self.measure.map(|(a, b)| Value::Number((b - a).length() as f64)).unwrap_or(Value::Null))
            .set("set", self.shown_set)
            .set("heatmap", match self.prefs.stretch {
                None => "off",
                Some(Measure::Area) => "area",
                Some(Measure::Angle) => "angle",
            })
            .set("export_dialog", self.export.open)
            .set("message", self.message.as_ref().map(|(text, _, _)| Value::String(text.clone())).unwrap_or(Value::Null))
            .set("shape", self.shape_state());
        if let Some((_, _, layout)) = &self.layout {
            // What is under the pointer, as the layout names it.
            let mut under = tequila_format::json_write::Object::new();
            under
                .set_some("island", self.hover.island.map(|i| i as usize))
                .set_some("face", self.hover.triangle.map(|t| t as usize))
                .set_some("edge", self.hover.edge.map(|e| Value::Array(layout.uv.edges[e as usize].vertices.iter().map(|&v| Value::Number(v as f64)).collect())))
                .set_some("vertex", self.hover.vertex.map(|v| layout.uv.uv_vertices[v as usize].vertex as usize));
            out.set("islands", numbers(editing::islands(layout, &self.selection)))
                .set("under", under.build())
                .set("island_count", layout.uv.islands.len())
                .set("seams", layout.seam_count())
                .set("edges", Value::Array(self.selection.edges().into_iter().map(|[a, b]| Value::Array(vec![Value::Number(a as f64), Value::Number(b as f64)])).collect()))
                .set("vertices", numbers(self.selection.vertices()));
        }
        if let Some(node) = self.hint.node {
            out.set("pins", numbers(self.pins.get(&(node, self.shown_set)).map(|p| p.iter().copied().collect()).unwrap_or_default()));
        }
        if let Some(param) = self.selection.texture() {
            out.set("texture", param.key());
        }
        if let Subject::Part(node) = self.subject {
            out.set("part", node.0);
        }
        out.build()
    }
}

/// A projected label put on the layout: filling the largest island, or a
/// quarter of the square where there is none.
fn on_layout(mut mapping: Mapping, layout: &layout::Layout) -> Mapping {
    mapping.projection = Projection::Uv;
    mapping.uv_set = layout.uv.set;
    mapping.real_world_units = false;
    mapping.rotation = 0.0;
    let biggest = (0..layout.points.len()).max_by(|&a, &b| layout.points[a].len().cmp(&layout.points[b].len()));
    mapping = placement::with_repeats(mapping, vec2(4.0, 4.0));
    match biggest {
        Some(island) => placement::fitted(mapping, &layout.points[island]),
        None => placement::centred(mapping, vec2(0.5, 0.5)),
    }
}

pub fn show(ctx: &egui::Context, editor: &mut UvEditor, project: &mut Project) {
    if !editor.open {
        editor.hovered = false;
        editor.hint = link::Hint::default();
        return;
    }
    // A label asked for: its part, the label selected, and a part with no
    // layout unwrapped first, as the button says. Of the parts that share
    // the material, the selected one, where the person is looking.
    if let Some((material, index)) = editor.requested.take() {
        let selected = project.selection.active().filter(|node| project.document().tree.get(*node).is_some_and(|n| n.material == Some(material)));
        if let Some(node) = selected.or_else(|| tequila_scene::decal_fit::part_of(project.document(), material).map(|(node, _)| node)) {
            editor.subject = Subject::Part(node);
            editor.set_mode(Mode::Labels);
            editor.selection.set(Item::Label(index));
            editor.frame_pending = true;
            let mesh = match project.document().tree.get(node).map(|n| &n.kind) {
                Some(NodeKind::Part { mesh }) => Some(*mesh),
                _ => None,
            };
            let set = project.document().materials.get(material).and_then(|m| m.graph.decals.get(index)).map(|d| if d.mapping.projection == Projection::Uv { d.mapping.uv_set } else { 0 }).unwrap_or(0);
            let laid_out = mesh.and_then(|key| project.document().geometry.get(key)).is_some_and(|mesh| mesh.uv_sets.get(set).is_some_and(|uv| uv.len() == mesh.positions.len()));
            editor.unwrap_pending = !laid_out;
        }
    }
    let Some(part) = editor.part(project) else {
        editor.hint = link::Hint::default();
        empty_window(ctx, editor);
        return;
    };
    if editor.unwrap_pending {
        editor.unwrap_pending = false;
        let options = tequila_scene::uv::UnwrapOptions { cone_degrees: editor.cone, method: editor.shape.settings.again.method, ..Default::default() };
        project.end_interaction();
        match project.apply(Command::UnwrapUvs { node: part.node, options }) {
            Ok(_) => project.status = format!("Unwrapped {} to place its label on the layout", part.name),
            Err(error) => project.status = format!("Could not unwrap {}: {error}", part.name),
        }
        project.end_interaction();
        editor.layout = None;
        editor.frame_pending = true;
    }
    // The part as it is after the unwrap.
    let Some(part) = editor.part(project) else { return };

    // Labels and textures that are gone - undone, removed elsewhere - let go.
    let (count, params): (usize, Vec<Param>) = (part.labels.len(), part.textures.iter().map(|t| t.param).collect());
    editor.selection.retain(|item| match item {
        Item::Label(i) => i < count,
        Item::Texture(p) => params.contains(&p),
        _ => true,
    });
    if editor.selected_guide.is_some_and(|g| g >= part.guides.len()) {
        editor.selected_guide = None;
    }

    // The islands, worked out once a mesh and set. A new mesh for the same
    // part and set is the layout edited: the view stays, and what was
    // selected is found again by the numbers edits keep.
    let set = editor.uv_set(&part);
    editor.shown_set = set;
    if editor.layout.as_ref().map(|(key, s, _)| (*key, *s)) != Some((part.mesh, set)) {
        let fresh = project.document().geometry.get(part.mesh).and_then(|mesh| layout::Layout::of(mesh, set));
        let same = editor.framed_for == Some((part.node, set));
        match (&fresh, editor.layout.as_ref().filter(|_| same)) {
            (Some(new), Some((_, _, old))) => {
                let vertices = new.vertex_uv.len();
                editor.selection.rename(|item| editing::renamed(new, item, vertices));
                editor.island = editor.island.and_then(|i| old.uv.islands.get(i)).and_then(|i| i.triangles.first()).and_then(|&t| new.uv.island_of.get(t as usize)).map(|&i| i as usize);
            }
            _ => {
                editor.island = None;
                editor.selection.retain(|item| !item.mode().is_layout());
                editor.frame_pending = true;
                editor.panel_preview = None;
            }
        }
        editor.framed_for = Some((part.node, set));
        editor.layout = fresh.map(|l| (part.mesh, set, l));
        editor.gesture = editor.gesture.take().filter(|g| !matches!(g, tools::Gesture::Layout { .. }));
    }
    // The pins on vertices that are gone - an undone cut - let go.
    if let Some(pins) = editor.pins.get_mut(&(part.node, set)) {
        let vertices = editor.layout.as_ref().map_or(0, |(_, _, l)| l.vertex_uv.len());
        pins.retain(|&v| (v as usize) < vertices);
    }
    // The stretch, while the heatmap or a selection of islands wants it.
    if editor.prefs.stretch.is_some() || editor.selection.is_layout() {
        if let Some(mesh) = project.document().geometry.get(part.mesh).filter(|_| editor.layout.is_some()) {
            editor.keep_distortion(mesh, part.mesh, set);
        }
    }
    // The face under the viewport's pointer, or a script's.
    if let Some(face) = editor.scripted.hover_face {
        let document = project.document();
        let at = document.geometry.get(part.mesh).filter(|m| (face as usize) < m.triangle_count()).map(|m| {
            let world = document.tree.world_transform(part.node);
            let corners = m.triangle(face as usize).map(|v| world.point(m.positions[v as usize]));
            (corners[0] + corners[1] + corners[2]) / 3.0
        });
        editor.viewport_hover = at.and_then(|at| link::hover_of(project, part.node, face, at));
    }
    let layout = editor.layout.take();
    let density = layout.as_ref().and_then(|(_, _, l)| l.density());
    editor.density = density;
    editor.scale = units::Scale::of(editor.prefs.unit, density);

    let mut edits = Edits::default();
    let mut open = true;
    let spec = crate::windows::Spec {
        kind: crate::windows::Kind::Uv,
        title: format!("UV editor — {}", part.name),
        size: editor.prefs.size,
        min_size: egui::vec2(720.0, 480.0),
        resizable: true,
        margin: 0.0,
    };
    let size = editor.prefs.size;
    let shown = crate::windows::show(
        ctx,
        spec,
        &mut open,
        |window| window.default_size(size).min_width(720.0).min_height(480.0).resizable(true),
        |ui| {
            let layout = layout.as_ref().map(|(_, _, l)| l);
            editor.body(ui, &part, layout, project, &mut edits);
            // Read here, from the window's own input: in a window of its
            // own, the main window's pointer is not the one held.
            let held = ui.input(|i| i.pointer.any_down());
            let native = ui.ctx().viewport_id() != egui::ViewportId::ROOT;
            let size = if native { ui.ctx().input(|i| i.viewport().inner_rect.map(|r| r.size())) } else { Some(ui.max_rect().size()) };
            (held, size)
        },
    );
    editor.layout = layout;
    editor.hovered = shown.hovered_inside;
    let (held, size) = shown.inner.unwrap_or((false, None));
    if !open {
        editor.open = false;
        editor.gesture = None;
        editor.help = false;
        editor.typing = None;
        editor.hint = link::Hint::default();
    }
    if let Some(size) = size.filter(|s| s.x >= 480.0 && s.y >= 360.0) {
        if (size - editor.prefs.size).length() > 1.0 && !held {
            editor.prefs.size = size;
        }
    }
    editor.apply(project, &part, edits, held);
    // Saved once a change has settled, at most once a second: a window
    // being resized changes its size every frame.
    let now = ctx.input(|i| i.time);
    if editor.prefs != editor.saved && !held && now - editor.saved_at >= 1.0 {
        editor.prefs.save();
        editor.saved = editor.prefs;
        editor.saved_at = now;
    }
}

/// The window with nothing to show yet.
fn empty_window(ctx: &egui::Context, editor: &mut UvEditor) {
    let mut open = true;
    let spec = crate::windows::Spec {
        kind: crate::windows::Kind::Uv,
        title: "UV editor".into(),
        size: editor.prefs.size,
        min_size: egui::vec2(720.0, 480.0),
        resizable: true,
        margin: 0.0,
    };
    let size = editor.prefs.size;
    let shown = crate::windows::show(ctx, spec, &mut open, |window| window.default_size(size).resizable(true), |ui| {
        ui.add_space(ui.available_height() * 0.25);
        crate::theme::empty_state(ui, Icon::Part, crate::theme::hue(crate::theme::Hue::Pink), "Select a part", "Select a part to see its UV layout, or open a label's UV editor from the Labels tab.");
    });
    editor.hovered = shown.hovered_inside;
    if !open {
        editor.open = false;
    }
}

impl UvEditor {
    fn body(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&layout::Layout>, project: &Project, edits: &mut Edits) {
        let p = crate::theme::p();
        let held = ui.input(|i| i.pointer.any_down());
        self.scripted_asks(part, layout, project, edits);
        egui::TopBottomPanel::top("uv-toolbar")
            .frame(egui::Frame::none().fill(p.chrome).inner_margin(egui::Margin::symmetric(8.0, 5.0)))
            .show_inside(ui, |ui| self.toolbar(ui, part, layout, edits));
        egui::TopBottomPanel::bottom("uv-status")
            .frame(egui::Frame::none().fill(p.chrome).inner_margin(egui::Margin::symmetric(10.0, 4.0)))
            .show_inside(ui, |ui| self.status_bar(ui, part, layout));
        egui::SidePanel::right("uv-side")
            .resizable(true)
            .default_width(340.0)
            .width_range(280.0..=520.0)
            .frame(egui::Frame::none().fill(p.panel).inner_margin(egui::Margin::same(8.0)))
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.side_panel(ui, part, layout, project, edits));
            });
        egui::CentralPanel::default().frame(egui::Frame::none().fill(p.panel)).show_inside(ui, |ui| match layout {
            Some(layout) => {
                if let Some(zoom) = self.zoom_request.take() {
                    if self.canvas_rect.is_positive() {
                        self.view.zoom_to(self.canvas_rect, zoom);
                        if (zoom - 1.0).abs() < 1e-6 {
                            self.view.pan = egui::Vec2::ZERO;
                        }
                    } else {
                        self.zoom_request = Some(zoom);
                    }
                }
                self.canvas(ui, part, layout, edits, project);
            }
            None => {
                self.hint = link::Hint { node: Some(part.node), ..Default::default() };
                ui.add_space(ui.available_height() * 0.2);
                crate::theme::empty_state(ui, Icon::Grid, crate::theme::hue(crate::theme::Hue::Pink), "No UV layout yet", "Unwrap the part to lay its surface out flat, then place labels and textures on it here.");
                ui.vertical_centered(|ui| {
                    if ui.button("Unwrap").on_hover_text("Lay the part's surface out flat as its first UV set").clicked() {
                        edits.unwrap = true;
                    }
                });
            }
        });

        // A script's align, as the panel's buttons do it.
        if let (Some(how), Some(layout)) = (self.scripted_align.take(), layout) {
            self.align(part, edits, layout, how);
        }

        // The keys, while the editor is under the pointer or has the
        // keyboard, and no field is being typed into.
        let native = ui.ctx().viewport_id() != egui::ViewportId::ROOT;
        let active = if native { ui.ctx().input(|i| i.viewport().focused.unwrap_or(true)) } else { ui.ui_contains_pointer() };
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        if active && !typing {
            if let Some(layout) = layout {
                for action in shortcuts::read(ui.ctx()) {
                    self.shortcut(action, part, layout, edits);
                }
            }
        }
        // The shape tools asked for this frame, by any of the ways there are.
        self.run_shape_tools(part, layout, project, edits);
        if self.help {
            self.help_popover(ui.ctx());
        }
        let mesh = project.document().geometry.get(part.mesh).filter(|_| layout.is_some()).map(|m| (part.mesh, m));
        if let export::Asked::Save = self.export.show(ui.ctx(), mesh, self.shown_set) {
            if let Some(path) = export::pick_path(&part.name, self.export.format) {
                self.write_export(project, part, self.shown_set, &path);
            }
        }
        if let Some(mut typing) = self.typing.take() {
            match guides::show(ui.ctx(), &mut typing, self.scale) {
                guides::Typed::Open => self.typing = Some(typing),
                guides::Typed::Cancelled => {}
                guides::Typed::Place(at) => {
                    let mut list = edits.guides.clone().unwrap_or_else(|| part.guides.clone());
                    if let Some(guide) = list.get_mut(typing.index) {
                        guide.at = at;
                        edits.guides = Some(list);
                    }
                }
            }
        }
        // A change typed or dragged into the Transform panel, sent once the
        // field is let go.
        if !held {
            if let (Some(map), Some(layout)) = (self.panel_preview.take(), layout) {
                self.transform_layout(part, layout, map, edits, "Moved");
            }
        }
    }

    /// Make this frame's edits commands: one step to undo for all of it, and
    /// a drag of a field one step however many frames it took.
    fn apply(&mut self, project: &mut Project, part: &Part, edits: Edits, held: bool) {
        if let Some(faces) = &edits.faces {
            // Faces picked as if in the viewport, at Face level.
            project.selection.set_level(tequila_scene::selection::SelectionLevel::Face);
            project.selection.clear();
            for (i, &face) in faces.iter().enumerate() {
                let mode = if i == 0 { tequila_scene::selection::SelectionMode::Replace } else { tequila_scene::selection::SelectionMode::Add };
                project.selection.click_face(part.node, face, mode);
            }
        }
        if edits.unwrap {
            let options = tequila_scene::uv::UnwrapOptions { cone_degrees: self.cone, method: self.shape.settings.again.method, ..Default::default() };
            project.end_interaction();
            match project.apply(Command::UnwrapUvs { node: part.node, options }) {
                Ok(_) => project.status = format!("Unwrapped {}", part.name),
                Err(error) => project.status = format!("Could not unwrap {}: {error}", part.name),
            }
            project.end_interaction();
            self.layout = None;
            self.framed_for = None;
            self.island = None;
            self.frame_pending = true;
        }
        let layout_edits = !edits.layout.is_empty();
        let labels = edits.labels.filter(|labels| *labels != part.labels);
        let guides = edits.guides.filter(|guides| *guides != part.guides);
        let mut commands = Vec::new();
        let mut refused = None;
        if let (Some(labels), Some(material)) = (labels.clone(), part.material) {
            if labels.iter().all(Decal::is_valid) {
                commands.push(Command::SetDecals { material, decals: labels });
            } else {
                refused = Some("That would leave a label too small or out of range".to_string());
            }
        }
        if let Some(material) = part.material {
            for (param, mapping) in edits.textures {
                commands.push(Command::SetTextureMapping { material, param, mapping: Box::new(mapping) });
            }
        }
        if let Some(guides) = guides {
            commands.push(Command::SetUvGuides { node: part.node, guides });
        }
        if layout_edits {
            // The layout's edits, with the labels on the islands they move
            // carried along, and whatever else changed: one step, or none
            // of it where the engine refuses one.
            project.end_interaction();
            project.begin_group();
            let mut done = Vec::new();
            let carry = edits.layout.iter().all(|edit| edit.carry);
            for edit in edits.layout {
                if let Err(error) = project.apply(edit.command) {
                    refused = Some(format!("Could not {}: {error}", edit.verb));
                    break;
                }
                if !done.contains(&edit.done) {
                    done.push(edit.done);
                }
            }
            if let Some(why) = refused {
                project.abandon_group();
                project.end_interaction();
                project.status = why.clone();
                self.say(&why, true);
                return;
            }
            let carried = if carry { self.carried_labels(project, part, labels.as_ref().unwrap_or(&part.labels)) } else { None };
            if let Some(carried) = carried {
                commands.retain(|c| !matches!(c, Command::SetDecals { .. }));
                if let Some(material) = part.material {
                    commands.insert(0, Command::SetDecals { material, decals: carried });
                }
            }
            for command in commands {
                if let Err(error) = project.apply(command) {
                    refused = Some(error.to_string());
                }
            }
            project.end_group();
            project.end_interaction();
            if let Some(show) = edits.show_set {
                self.show_set(show);
            }
            let said = done.join("; ");
            project.status = refused.clone().unwrap_or_else(|| said.clone());
            self.say(refused.as_deref().unwrap_or(&said), refused.is_some());
        } else if !commands.is_empty() || refused.is_some() {
            // A field dragged, while the button is held, edits the same
            // labels frame after frame: those merge into one step, which a
            // group would keep apart. A group only for several at once.
            if !held {
                project.end_interaction();
            }
            let group = commands.len() > 1;
            if group {
                project.begin_group();
            }
            for command in commands {
                if let Err(error) = project.apply(command) {
                    refused = Some(error.to_string());
                }
            }
            if group {
                project.end_group();
            }
            if !held {
                project.end_interaction();
            }
            if let Some(error) = refused {
                self.say(&error, true);
                project.status = error;
            }
        } else if !held {
            project.end_interaction();
        }
        if let Some(status) = edits.status {
            self.say(&status, false);
            project.status = status;
        }
        if edits.add_label {
            if let (Some(material), Some(path)) = (part.material, pick_artwork()) {
                self.add_label(project, part, material, &path);
            }
        }
    }

    /// The labels on islands an edit just moved, moved with them: each
    /// label whose middle was on an island goes where that island went -
    /// moved, turned, scaled, mirrored - so it prints where it printed. None
    /// where no label moved.
    fn carried_labels(&self, project: &Project, part: &Part, labels: &[Decal]) -> Option<Vec<Decal>> {
        let (_, set, before) = self.layout.as_ref()?;
        let mesh = project.document().tree.get(part.node).and_then(|n| n.mesh())?;
        let after_mesh = project.document().geometry.get(mesh)?;
        if after_mesh.triangle_count() != before.triangles.len() || after_mesh.uv_sets.get(*set).is_none_or(|uv| uv.len() != after_mesh.positions.len()) {
            return None;
        }
        let after = |t: usize| after_mesh.triangle(t).map(|v| after_mesh.uv_sets[*set][v as usize]);
        let mut out = labels.to_vec();
        let mut moved = false;
        for label in out.iter_mut() {
            if label.mapping.projection != Projection::Uv || label.mapping.uv_set != *set || label.locked {
                continue;
            }
            let Some(island) = before.at(placement::centre(&label.mapping)) else { continue };
            // An island's corners before and after, a few hundred at most.
            let triangles = &before.uv.islands[island].triangles;
            let step = (triangles.len() / 300).max(1);
            let (mut from, mut to) = (Vec::new(), Vec::new());
            for &t in triangles.iter().step_by(step) {
                from.extend_from_slice(&before.triangles[t as usize]);
                to.extend_from_slice(&after(t as usize));
            }
            let Some(likeness) = editing::likeness(&from, &to) else { continue };
            if likeness.is_still() {
                continue;
            }
            label.mapping = placement::carried(label.mapping, &likeness);
            moved = true;
        }
        moved.then_some(out)
    }

    /// A label from an image file, as the Labels tab adds one, then placed
    /// on the layout: one step to undo.
    fn add_label(&mut self, project: &mut Project, part: &Part, material: MaterialKey, path: &std::path::Path) {
        let command = match crate::decal_tab::label_from_file(material, path) {
            Ok((command, _)) => command,
            Err(error) => {
                project.status = error;
                return;
            }
        };
        project.end_interaction();
        project.begin_group();
        let added = project.apply(command);
        if added.is_ok() {
            if let (Some(definition), Some((_, _, layout))) = (project.document().materials.get(material), self.layout.as_ref()) {
                let mut decals = definition.graph.decals.clone();
                if let Some(label) = decals.last_mut() {
                    label.mapping = on_layout(label.mapping, layout);
                    // Its own shape: the artwork's proportions, not a square.
                    if let Some(source) = label_aspect(project, label.texture) {
                        let size = placement::size(&label.mapping);
                        label.mapping = placement::resized(label.mapping, vec2(size.x, size.x / source.max(1e-3)));
                        if let Some(island) = (0..layout.points.len()).max_by_key(|&i| layout.points[i].len()) {
                            label.mapping = placement::fitted(label.mapping, &layout.points[island]);
                        }
                    }
                    let count = decals.len();
                    let _ = project.apply(Command::SetDecals { material, decals });
                    self.set_mode(Mode::Labels);
                    self.selection.set(Item::Label(count - 1));
                }
            }
        }
        project.end_group();
        project.end_interaction();
        project.status = match added {
            Ok(_) => format!("Added a label to {}", part.name),
            Err(error) => format!("Cannot add the label: {error}"),
        };
    }
}

/// The artwork's width over its height.
fn label_aspect(project: &Project, id: TextureId) -> Option<f32> {
    let source = project.document().textures.get(id)?;
    let image = tequila_texture::image::Image::load_raw(&source.path).ok()?;
    (image.height > 0).then(|| image.width as f32 / image.height as f32)
}

/// An image file to make a label of, chosen.
fn pick_artwork() -> Option<std::path::PathBuf> {
    let artwork = crate::decal_tab::artwork_extensions();
    rfd::FileDialog::new().add_filter("Artwork", &artwork).pick_file()
}

/// A label's artwork as a texture to draw, read once: at most a thousand
/// pixels across, which is finer than the editor shows it.
pub(super) fn artwork(cache: &mut HashMap<TextureId, Option<egui::TextureHandle>>, project: &Project, id: TextureId, ctx: &egui::Context) -> Option<egui::TextureHandle> {
    if let Some(cached) = cache.get(&id) {
        return cached.clone();
    }
    let loaded = project.document().textures.get(id).and_then(|source| {
        let image = tequila_texture::image::Image::load_raw(&source.path).ok()?;
        let step = (image.width.max(image.height) as f32 / 1024.0).ceil().max(1.0) as usize;
        let (width, height) = (image.width.div_ceil(step), image.height.div_ceil(step));
        let mut pixels = Vec::with_capacity(width * height);
        let channel = |x: usize, y: usize, c: usize| image.data[(y * image.width + x) * image.channels + c.min(image.channels - 1)];
        for y in 0..height {
            for x in 0..width {
                let (sx, sy) = ((x * step).min(image.width - 1), (y * step).min(image.height - 1));
                let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let (r, g, b) = if image.channels >= 3 { (channel(sx, sy, 0), channel(sx, sy, 1), channel(sx, sy, 2)) } else { let l = channel(sx, sy, 0); (l, l, l) };
                let a = if image.channels == 4 || image.channels == 2 { channel(sx, sy, image.channels - 1) } else { 1.0 };
                pixels.push(egui::Color32::from_rgba_unmultiplied(byte(r), byte(g), byte(b), byte(a)));
            }
        }
        let image = egui::ColorImage { size: [width, height], pixels };
        Some(ctx.load_texture(format!("uv-editor-artwork-{}", id.0), image, egui::TextureOptions::LINEAR))
    });
    cache.insert(id, loaded.clone());
    loaded
}
