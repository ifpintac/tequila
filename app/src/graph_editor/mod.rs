//! The Material Graph editor (`docs/04` §7).
//!
//! A canvas over [`MaterialGraph`], driven the way Blender's node editor is
//! driven - the same keys, the same mouse, the same Add menu - so a graph
//! built in one is built the same way in the other. Each node shows its
//! values in place and a picture of what it makes; beside the canvas, the
//! material is rendered large as it is edited, the way V-Ray's material
//! editor keeps one in view.
//!
//! Every edit is one `material.set_graph` command: undoable with Ctrl+Z like
//! anything else, journalled for crash recovery, and refused if it would not
//! compile, so a loop or a type mismatch is explained here rather than
//! reaching the renderer. A drag of a node or a slider is one undo step, not
//! one per frame.

mod aliases;
mod browser;
mod menu;
pub use browser::Setup;
pub use browser::Source as Brought;

/// What a script asks of the browser.
pub enum Asked {
    /// A material brought into the graph whole.
    Bring(Brought),
    /// A setup's nodes added, wired.
    Arrange(Setup),
}
mod previews;
mod sidebar;
mod swatch;
mod thumbs;
mod tidy;
mod view;

use crate::project::Project;
use tequila_core::math::Vec2 as GraphVec2;
use tequila_materials::graph::{Link, MaterialGraph, Node, NodeId, NodeKind, Socket, SocketType, Value};
use tequila_materials::material_type::Param;
use tequila_scene::tree::MaterialKey;
use tequila_scene::Command;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use view::{Inline, NodeLayout, RowKind, SettingField, SettingRef, Target, SOCKET_RADIUS};

/// The material library, as the browser lists it: its entries, and the
/// picture the library keeps of one, when it has it to hand.
pub struct Library<'a> {
    pub entries: &'a [tequila_format::library::LibraryEntry],
    pub picture: &'a dyn Fn(usize) -> Option<egui::ColorImage>,
}

impl Library<'_> {
    /// No library: the browser's last group is empty.
    #[cfg(test)]
    pub fn none() -> Library<'static> {
        Library { entries: &[], picture: &|_| None }
    }
}

/// One end of a wire being dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum End {
    Output(NodeId, u8),
    Input(NodeId, Socket),
}

#[derive(Clone, Debug)]
enum Drag {
    Pan,
    /// Nodes (and frames) being moved, from where each started.
    Move { start: egui::Vec2, nodes: Vec<(NodeId, GraphVec2)>, moved: bool },
    Wire { from: End },
    Box { start: egui::Pos2, add: bool, remove: bool },
    Cut { points: Vec<egui::Pos2> },
    Frame { index: usize, grab: egui::Vec2 },
    FrameResize { index: usize },
    /// A number field being scrubbed; a click without a drag types into it.
    Field { target: Target, start: f32, int: bool, range: Option<(f32, f32)>, travel: f32, moved: bool },
}

/// G: the selection follows the pointer until a click.
struct Grab {
    start: egui::Vec2,
    nodes: Vec<(NodeId, GraphVec2)>,
}

/// A field being typed into.
struct Typing {
    target: Target,
    text: String,
    focus: bool,
    rect: egui::Rect,
    /// Opened this frame (a colour's picker, by a press).
    fresh: bool,
}

struct Dropdown {
    node: NodeId,
    setting: SettingRef,
    at: egui::Pos2,
    width: f32,
    /// Opened this frame, by the press that a click outside would be.
    fresh: bool,
}

struct ContextMenu {
    node: Option<NodeId>,
    at: egui::Pos2,
    canvas: egui::Vec2,
}

/// What Ctrl+C took.
#[derive(Clone)]
struct Clipboard {
    nodes: Vec<(NodeId, Node)>,
    links: Vec<Link>,
}

pub struct GraphEditor {
    pub open: bool,
    material: Option<MaterialKey>,
    pan: egui::Vec2,
    zoom: f32,
    selected: BTreeSet<NodeId>,
    /// The node last clicked: the one the sidebar shows.
    active: Option<NodeId>,
    selected_frame: Option<usize>,
    drag: Option<Drag>,
    grab: Option<Grab>,
    menu: Option<menu::AddMenu>,
    dropdown: Option<Dropdown>,
    context: Option<ContextMenu>,
    typing: Option<Typing>,
    previews: previews::Previews,
    swatch: swatch::Swatch,
    show_previews: bool,
    message: Option<String>,
    /// Fit the view to the graph on the next frame.
    frame_pending: bool,
    /// Whether the pointer was over the editor last frame. Global shortcuts
    /// that mean something here step aside while it is, so Delete deletes a
    /// node here and not a part.
    pub hovered: bool,
    /// The nodes a frame being dragged carries, and where each sits in it.
    frame_members: Vec<(NodeId, egui::Vec2)>,
    /// A solo asked for from the sidebar or a click, which cannot reach the
    /// project mutably: `Some(None)` ends it.
    solo_request: Option<Option<(NodeId, u8)>>,
    clipboard: Option<Clipboard>,
    /// The wire a node being dragged would be dropped into.
    insert_into: Option<Link>,
    /// Focus the name field in the sidebar (F2).
    focus_label: bool,
    /// The node and output soloed from here, to cycle through outputs.
    solo_shown: Option<(NodeId, u8)>,
    /// Where the canvas was last drawn, for the toolbar above it.
    canvas_rect: egui::Rect,
    /// The overview in the canvas's corner, when the graph does not fit.
    show_map: bool,
    /// Where it was drawn last frame, and how it maps the canvas: so the
    /// canvas does not take a click meant for it.
    minimap: Option<Minimap>,
    /// The span it shows, held while it is dragged so it does not rescale
    /// under the pointer.
    map_world: Option<egui::Rect>,
    /// The Material/Map browser down the left side, and whether it shows:
    /// the toolbar's Browser, T, and Preferences › Interface.
    browser: browser::Browser,
    pub show_browser: bool,
    /// Whether the swatch and the settings show, down the right side (N).
    show_settings: bool,
    /// A scene material clicked in the browser: the editor opens on it once
    /// this frame's edits to the one it is on are applied.
    switch_to: Option<MaterialKey>,
    /// The pictures a library material brought into the graph reads, which
    /// the document does not have yet: added with the graph, as one step.
    pending_textures: Vec<(tequila_materials::graph::TextureId, tequila_scene::textures::TextureSource)>,
    /// A row of the browser is held over the canvas, and `insert_into` is
    /// its wire, not a dragged node's.
    browser_holding: bool,
    /// What a script asked of the browser, for the next frame.
    scripted: Option<browser::Act>,
    /// The small pictures beside the browser's rows.
    thumbs: thumbs::Thumbs,
}

/// The minimap's place on screen and its scale: a canvas point `p` is at
/// `offset + p * scale`.
#[derive(Clone, Copy, Debug)]
struct Minimap {
    rect: egui::Rect,
    offset: egui::Vec2,
    scale: f32,
}

impl Minimap {
    fn to_map(self, p: egui::Pos2) -> egui::Pos2 {
        egui::pos2(self.offset.x + p.x * self.scale, self.offset.y + p.y * self.scale)
    }

    fn to_canvas(self, p: egui::Pos2) -> egui::Pos2 {
        egui::pos2((p.x - self.offset.x) / self.scale, (p.y - self.offset.y) / self.scale)
    }
}

impl Default for GraphEditor {
    fn default() -> Self {
        GraphEditor {
            open: false,
            material: None,
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
            selected: BTreeSet::new(),
            active: None,
            selected_frame: None,
            drag: None,
            grab: None,
            menu: None,
            dropdown: None,
            context: None,
            typing: None,
            previews: previews::Previews::default(),
            swatch: swatch::Swatch::default(),
            show_previews: true,
            message: None,
            frame_pending: false,
            hovered: false,
            frame_members: Vec::new(),
            solo_request: None,
            clipboard: None,
            insert_into: None,
            focus_label: false,
            solo_shown: None,
            canvas_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            show_map: true,
            minimap: None,
            map_world: None,
            browser: browser::Browser::default(),
            show_browser: true,
            show_settings: true,
            switch_to: None,
            pending_textures: Vec::new(),
            browser_holding: false,
            scripted: None,
            thumbs: thumbs::Thumbs::default(),
        }
    }
}

/// The canvas's colours.
/// From the theme: a dark canvas in dark mode, a light one in light, with
/// node headers in their family's colour either way.
mod ink {
    use crate::theme::p;
    use eframe::egui::Color32;
    pub fn canvas() -> Color32 {
        p().canvas
    }
    pub fn dot() -> Color32 {
        p().canvas_dot
    }
    pub fn dot_major() -> Color32 {
        p().canvas_dot_major
    }
    pub fn body() -> Color32 {
        p().node_body
    }
    pub fn field() -> Color32 {
        p().node_field
    }
    pub fn field_hover() -> Color32 {
        p().node_field_hover
    }
    pub fn fill() -> Color32 {
        p().node_fill
    }
    pub fn text() -> Color32 {
        p().node_text
    }
    pub fn label() -> Color32 {
        p().node_label
    }
    /// A title on a coloured header: light in both modes.
    pub fn header_text() -> Color32 {
        Color32::from_gray(240)
    }
    pub const MUTED: Color32 = Color32::from_rgb(214, 86, 74);
    /// The active node's ring: white on dark, ink on light.
    pub fn active() -> Color32 {
        if p().dark { Color32::WHITE } else { Color32::from_gray(24) }
    }
    /// Shadows under nodes and wires.
    pub fn shade(dark: u8) -> Color32 {
        Color32::from_black_alpha(if p().dark { dark } else { dark / 3 })
    }
}

impl GraphEditor {
    pub fn open_for(&mut self, material: MaterialKey) {
        if self.material != Some(material) {
            self.selected.clear();
            self.active = None;
            self.selected_frame = None;
            // A solo belongs to the material it was made on.
            self.solo_request = Some(None);
            self.previews.clear();
            self.pan = egui::vec2(40.0, 40.0);
            self.zoom = 1.0;
        }
        self.material = Some(material);
        self.open = true;
        self.frame_pending = true;
        self.message = None;
    }

    /// Close on the document being replaced: its material key, selection
    /// and previews meant that document.
    pub fn forget_document(&mut self) {
        self.open = false;
        self.material = None;
        self.selected.clear();
        self.active = None;
        self.selected_frame = None;
        self.previews.clear();
    }

    /// The material the editor was last opened on.
    pub fn material(&self) -> Option<MaterialKey> {
        self.material
    }

    /// Type in the browser's search, showing the browser: a script's way
    /// to a picture of what it finds.
    pub fn search_browser(&mut self, query: &str) {
        self.browser.query = query.to_string();
        self.show_browser = true;
    }

    #[cfg(test)]
    pub fn browser_search(&self) -> &str {
        &self.browser.query
    }

    /// Bring a material into the graph whole, or add a setup, as its row
    /// in the browser double-clicked: a script's way, done when the editor
    /// is next drawn.
    pub fn ask(&mut self, asked: Asked) {
        self.scripted = Some(match asked {
            Asked::Bring(source) => browser::Act::Bring { source, at: None },
            Asked::Arrange(setup) => browser::Act::Arrange { setup, at: None },
        });
    }

    /// `library` is the material library, for the browser's last group.
    pub fn show(&mut self, ctx: &egui::Context, project: &mut Project, library: &Library) {
        if !self.open {
            self.hovered = false;
            return;
        }
        let Some(key) = self.material else { return };
        let Some(definition) = project.document().materials.get(key) else {
            self.open = false;
            project.set_solo(None);
            return;
        };
        // A solo on a node that has gone ends with it.
        let stale = project
            .solo()
            .is_some_and(|(material, node, _)| material != key || definition.graph.node(node).is_none());
        if stale {
            project.set_solo(None);
        }
        let Some(definition) = project.document().materials.get(key) else {
            return;
        };
        let title = format!("Material graph — {}", definition.name);
        let name = definition.name.clone();
        let exposed = definition.material_type.exposed();
        let original = definition.graph.clone();
        // A graph nobody laid out is shown laid out, but not changed for
        // being looked at: the positions are saved with the first real edit.
        // (Not while a node is being moved: it may pass exactly over
        // another on its way, and is stepped off one it is let go on.)
        let baseline = if self.drag.is_none() && self.grab.is_none() && tidy::unplaced(&original) {
            let mut arranged = original.clone();
            tidy::arrange(&mut arranged, &exposed);
            arranged
        } else {
            original
        };
        let mut graph = baseline.clone();
        if self.zoom <= 0.0 {
            self.zoom = 1.0;
        }
        self.previews.collect(ctx);

        let mut open = true;
        let mut committed = false;
        let spec = crate::windows::Spec {
            kind: crate::windows::Kind::Graph,
            title,
            // Wide enough for the browser, the canvas and the settings.
            size: egui::vec2(1400.0, 760.0),
            min_size: egui::vec2(720.0, 420.0),
            resizable: true,
            margin: 0.0,
        };
        let window = crate::windows::show(ctx, spec, &mut open, |window| window.default_size([1400.0, 760.0]).min_width(720.0).min_height(420.0).resizable(true), |ui| {
                // A script's request of the browser, whether it shows or not.
                if let Some(act) = self.scripted.take() {
                    committed |= self.browse(act, &mut graph, project, library, &exposed);
                }
                if self.show_browser {
                    egui::SidePanel::left("graph-browser")
                        .resizable(true)
                        .default_width(224.0)
                        .width_range(180.0..=360.0)
                        .show_inside(ui, |ui| {
                            committed |= crate::profile::time("graph: browser", || self.browser_panel(ui, project, library, &mut graph, &exposed, key));
                        });
                }
                if self.show_settings {
                    egui::SidePanel::right("graph-properties")
                        .resizable(true)
                        .default_width(300.0)
                        .width_range(240.0..=460.0)
                        .show_inside(ui, |ui| {
                            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                                ui.add_space(4.0);
                                crate::profile::time("graph: swatch", || self.swatch_panel(ui, &graph, project));
                                ui.add_space(4.0);
                                ui.separator();
                                committed |= crate::profile::time("graph: properties", || self.properties(ui, project, &mut graph));
                            });
                        });
                }
                egui::TopBottomPanel::bottom("graph-status").show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        let text = match &self.message {
                            Some(message) => egui::RichText::new(message).color(crate::theme::p().danger),
                            // What there is and how close, and one way in.
                            // It went on to list six shortcuts the sidebar
                            // lists too, and said Ctrl where a Mac has Cmd.
                            None => egui::RichText::new(status_line(graph.len(), self.selected.len(), self.zoom)).small().color(crate::theme::dim()),
                        };
                        ui.label(text);
                    });
                });
                egui::TopBottomPanel::top("graph-toolbar")
                    .frame(egui::Frame::none().fill(crate::theme::p().chrome).inner_margin(egui::Margin::symmetric(8.0, 5.0)))
                    .show_inside(ui, |ui| {
                        self.toolbar(ui, &name, &mut graph, &exposed, &mut committed);
                    });
                egui::CentralPanel::default().frame(egui::Frame::none().fill(ink::canvas())).show_inside(ui, |ui| {
                    committed |= crate::profile::time("graph: canvas", || self.canvas(ui, project, &mut graph, &exposed));
                });
                // Read here, from the window's own input: in a window of its
                // own, the main window's pointer is not the one dragging.
                ui.input(|i| (i.pointer.any_down(), i.pointer.any_released()))
            });
        self.hovered = window.hovered_inside;
        let (pointer_down, pointer_released) = window.inner.unwrap_or((false, false));
        if let Some(request) = self.solo_request.take() {
            project.set_solo(request.map(|(node, output)| (key, node, output)));
        }
        if !open {
            self.open = false;
            self.menu = None;
            self.grab = None;
            // A solo is a look at one node while editing; closing the editor
            // puts the material back.
            project.set_solo(None);
        }

        if graph != baseline {
            // Continuous edits (a node being dragged, a slider held) merge
            // into one step; a finished one closes it.
            let dragging = pointer_down || self.grab.is_some();
            // A library material brought in: its pictures first, and the
            // graph that reads them, as one step.
            let textures = std::mem::take(&mut self.pending_textures);
            let grouped = !textures.is_empty();
            if grouped {
                project.end_interaction();
                project.begin_group();
                let payload = tequila_scene::command::ImportPayload { meshes: Vec::new(), textures, materials: Vec::new(), nodes: Vec::new(), roots: Vec::new() };
                if let Err(error) = project.apply(Command::Import { payload: Box::new(payload) }) {
                    self.message = Some(explain(&error.to_string()));
                }
            }
            match project.apply(Command::SetGraph { material: key, graph: Box::new(graph) }) {
                Ok(()) => self.message = None,
                Err(error) => self.message = Some(explain(&error.to_string())),
            }
            if grouped {
                project.end_group();
            }
            if !dragging || committed {
                project.end_interaction();
            }
        } else if pointer_released {
            project.end_interaction();
        }
        // Another of the scene's materials, clicked in the browser: after
        // the edits above, which were to the one the editor was on.
        if let Some(next) = self.switch_to.take() {
            if self.open && project.document().materials.get(next).is_some() {
                project.end_interaction();
                self.open_for(next);
            }
        }
    }

    // ---- the browser ------------------------------------------------------------------

    /// The Material/Map browser, down the left side.
    fn browser_panel(
        &mut self,
        ui: &mut egui::Ui,
        project: &Project,
        library: &Library,
        graph: &mut MaterialGraph,
        exposed: &[Param],
        key: MaterialKey,
    ) -> bool {
        let entries = crate::profile::time("graph: entries", || menu::entries(project));
        let materials: Vec<(MaterialKey, String)> = project.document().materials.iter().map(|(key, definition)| (key, definition.name.clone())).collect();
        let shelf: Vec<(&str, &str)> = library.entries.iter().map(|entry| (entry.name.as_str(), entry.category.as_str())).collect();
        let sections = crate::profile::time("graph: sections", || browser::sections(&entries, &materials, &shelf, Some(key), &self.browser.query));
        // What each row's picture is kept under: the material's graph, the
        // library's own key, a node as it is first set.
        let pictured = |item: &browser::Item| -> Option<u64> {
            match item {
                browser::Item::Node(NodeKind::Shader(material), _) => Some(thumbs::key_of(material, 1)),
                browser::Item::Node(kind, _) => matches!(view::family(kind), view::Family::Texture | view::Family::Pattern).then(|| thumbs::key_of(kind, 2)),
                browser::Item::Material(key) => project.document().materials.get(*key).map(|definition| definition.graph.content_hash() ^ 0x5ce9e),
                browser::Item::Library(index) => library.entries.get(*index).map(|entry| crate::library_tab::entry_key(entry) ^ 0x11b),
                browser::Item::Setup(_) => None,
            }
        };
        self.thumbs.collect(ui.ctx());
        let act = crate::profile::time("graph: browser rows", || self.browser.show(ui, &sections, self.canvas_rect, &mut self.thumbs, &pictured));
        // The rows that came into view without one: the library's own
        // picture if it has it, and the worker for the rest.
        let wanted = self.thumbs.take_wanted();
        if !wanted.is_empty() {
            for row in sections.iter().flat_map(|section| &section.rows) {
                let Some(key) = pictured(&row.item).filter(|key| wanted.contains(key)) else { continue };
                let what = match &row.item {
                    browser::Item::Node(NodeKind::Shader(material), _) => {
                        Some(thumbs::Make::Ball { graph: MaterialGraph::from_material((**material).clone()), files: Vec::new(), loaded: None, uv_unit: None })
                    }
                    browser::Item::Node(kind, _) => Some(thumbs::Make::Map { kind: kind.clone(), textures: project.scene().textures.clone() }),
                    browser::Item::Material(material) => project
                        .document()
                        .materials
                        .get(*material)
                        .map(|definition| thumbs::Make::Ball { graph: definition.graph.clone(), files: Vec::new(), loaded: Some(project.scene().textures.clone()), uv_unit: None }),
                    browser::Item::Library(index) => match (library.picture)(*index) {
                        Some(image) => {
                            self.thumbs.put(ui.ctx(), key, image);
                            continue;
                        }
                        None => library.entries.get(*index).map(|entry| thumbs::Make::Ball { graph: entry.graph.clone(), files: entry.textures.clone(), loaded: None, uv_unit: entry.preview_uv_unit }),
                    },
                    browser::Item::Setup(_) => None,
                };
                match what {
                    Some(what) => self.thumbs.make(ui.ctx(), key, what),
                    None => self.thumbs.none(key),
                }
            }
        }
        // A node's row held over a wire it could go into: the wire lights,
        // as it does under a node dragged on the canvas.
        let pointer = ui.input(|i| i.pointer.latest_pos()).filter(|at| self.canvas_rect.contains(*at));
        match (self.browser.dragged_node().cloned(), pointer) {
            (Some(kind), Some(at)) => {
                self.insert_into = self.wire_at(graph, exposed, &kind, at);
                self.browser_holding = true;
            }
            _ if self.browser_holding => {
                self.insert_into = None;
                self.browser_holding = false;
            }
            _ => {}
        }
        self.browse(act, graph, project, library, exposed)
    }

    /// Where a node whose header is under `screen` sits on the canvas.
    fn dropped_at(&self, kind: &NodeKind, screen: egui::Pos2) -> egui::Vec2 {
        self.to_canvas(self.canvas_rect, screen) - egui::vec2(view::node_width(kind) * 0.5, 12.0)
    }

    /// The wire a node of `kind` dropped at `screen` would go into.
    fn wire_at(&self, graph: &MaterialGraph, exposed: &[Param], kind: &NodeKind, screen: egui::Pos2) -> Option<Link> {
        let rect = self.canvas_rect;
        let at = self.dropped_at(kind, screen);
        // Its header and a little of its body: what the pointer carries.
        let held = egui::Rect::from_min_size(at.to_pos2(), egui::vec2(view::node_width(kind), 60.0));
        let layouts = tidy::layouts(graph, exposed, self.show_previews);
        self.wire_fitting(graph, &layouts, rect, self.rect_to_screen(rect, held), kind, None)
    }

    /// Do what the browser was asked: true when the graph changed.
    fn browse(&mut self, act: browser::Act, graph: &mut MaterialGraph, project: &Project, library: &Library, exposed: &[Param]) -> bool {
        let rect = self.canvas_rect;
        // Double-clicked: the middle of the view, each a step down and
        // right of the last so none hides another.
        let middle = |editor: &GraphEditor, width: f32| editor.to_canvas(rect, rect.center()) - egui::vec2(width * 0.5, 60.0);
        match act {
            browser::Act::None => false,
            browser::Act::Open(material) => {
                self.switch_to = Some(material);
                false
            }
            browser::Act::Add { kind, label, at } => {
                // Dropped on a wire it fits: it goes in between.
                let link = at.and_then(|screen| self.wire_at(graph, exposed, &kind, screen));
                let at = match at {
                    Some(screen) => self.dropped_at(&kind, screen),
                    // Clear of the nodes that are there, if a few steps find
                    // such a place.
                    None => {
                        let wanted = middle(self, view::node_width(&kind));
                        let size = egui::vec2(view::node_width(&kind), 120.0);
                        let taken: Vec<egui::Rect> = tidy::layouts(graph, exposed, self.show_previews).into_values().map(|l| l.rect.expand(GRID)).collect();
                        (0..16)
                            .map(|step| wanted + egui::Vec2::splat(step as f32 * GRID * 4.0))
                            .find(|at| !taken.iter().any(|r| r.intersects(egui::Rect::from_min_size(at.to_pos2(), size))))
                            .unwrap_or(wanted)
                    }
                };
                let mut node = Node::new(kind);
                node.label = label;
                node.position = GraphVec2 { x: snap(at.x), y: snap(at.y) };
                let id = graph.add_node(node);
                if let Some(link) = link {
                    insert(graph, link, id);
                }
                apart(graph, &[id]);
                self.insert_into = None;
                self.browser_holding = false;
                self.selected = BTreeSet::from([id]);
                self.active = Some(id);
                self.selected_frame = None;
                true
            }
            browser::Act::Arrange { setup, at } => {
                let (mut other, into) = setup_graph(setup);
                if setup == browser::Setup::StochasticTiling {
                    let picture = picture_for_setup(graph, self.active, project);
                    let images: Vec<NodeId> = other.nodes().filter(|(_, n)| matches!(n.kind, NodeKind::ImageTexture { .. })).map(|(id, _)| id).collect();
                    for id in images {
                        if let Some(NodeKind::ImageTexture { texture, .. }) = other.node_mut(id).map(|n| &mut n.kind) {
                            *texture = picture;
                        }
                    }
                }
                tidy::arrange(&mut other, &[]);
                let top = NodeKind::MixShader { add: false };
                let dropped = at.is_some();
                // The node that feeds the surface: the one the rest are
                // placed about.
                let stand_in = other.output();
                let last = other.links().iter().find(|l| l.to == stand_in).and_then(|l| other.node(l.from)).map(|n| n.position).unwrap_or(GraphVec2 { x: 0.0, y: 0.0 });
                let at = match at {
                    Some(screen) => self.dropped_at(&top, screen),
                    // To the left of everything that is there, level with
                    // the material: where what feeds it comes from, so its
                    // wire runs straight in.
                    None => {
                        let here = tidy::content_bounds(graph, exposed, self.show_previews);
                        let theirs = tidy::layouts(&other, &[], self.show_previews).into_iter().filter(|(id, _)| *id != stand_in).map(|(_, l)| l.rect).reduce(|a, b| a.union(b));
                        let level = graph.node(graph.output()).map_or(0.0, |n| n.position.y);
                        match (here, theirs) {
                            (Some(here), Some(theirs)) => egui::vec2(last.x + (here.left() - GRID * 10.0 - theirs.right()), last.y + (level - theirs.top())),
                            _ => middle(self, view::node_width(&top)),
                        }
                    }
                };
                let fresh = arrange_setup(graph, &other, into, GraphVec2 { x: at.x, y: at.y });
                apart(graph, &fresh);
                let layouts = tidy::layouts(graph, exposed, self.show_previews);
                if let Some(bounds) = fresh.iter().filter_map(|id| layouts.get(id).map(|l| l.rect)).reduce(|a, b| a.union(b)) {
                    let around = bounds.expand(24.0);
                    graph.frames.push(tequila_materials::graph::Frame {
                        title: setup.title().into(),
                        min: GraphVec2 { x: around.min.x, y: around.min.y - tidy::FRAME_TITLE },
                        max: GraphVec2 { x: around.max.x, y: around.max.y },
                    });
                }
                if !dropped {
                    self.frame_pending = true;
                }
                self.active = fresh.last().copied();
                self.selected = fresh.into_iter().collect();
                self.selected_frame = None;
                true
            }
            browser::Act::Bring { source, at } => {
                // The material as this document holds it: a library
                // material's pictures numbered as the document's are.
                let (name, mut other, its_exposed, textures) = match source {
                    browser::Source::Scene(key) => {
                        let Some(definition) = project.document().materials.get(key) else { return false };
                        (definition.name.clone(), definition.graph.clone(), definition.material_type.exposed(), Vec::new())
                    }
                    browser::Source::Library(index) => {
                        let Some(entry) = library.entries.get(index) else { return false };
                        let (definition, textures) = entry.for_document(project.document());
                        (entry.name.clone(), definition.graph, entry.material_type.exposed(), textures)
                    }
                };
                // One nobody laid out comes in laid out, not in a heap.
                if tidy::unplaced(&other) {
                    tidy::arrange(&mut other, &its_exposed);
                }
                let shader = NodeKind::Shader(Box::default());
                let dropped = at.is_some();
                let at = match at {
                    Some(screen) => self.dropped_at(&shader, screen),
                    // Under everything that is there, from its left edge:
                    // a material's nodes need room no middle of a view has.
                    None => {
                        let here = tidy::content_bounds(graph, exposed, self.show_previews);
                        let theirs = tidy::content_bounds(&other, &its_exposed, self.show_previews);
                        let surface = other.node(other.output()).map(|n| n.position).unwrap_or(GraphVec2 { x: 0.0, y: 0.0 });
                        match (here, theirs) {
                            (Some(here), Some(theirs)) => egui::vec2(here.left() + (surface.x - theirs.left()), here.bottom() + GRID * 8.0 + tidy::FRAME_TITLE + (surface.y - theirs.top())),
                            _ => middle(self, view::node_width(&shader)),
                        }
                    }
                };
                let (shader, fresh) = bring_in(graph, &other, &name, GraphVec2 { x: at.x, y: at.y });
                apart(graph, &fresh);
                // Put below what was in view: the view takes it all in.
                if !dropped {
                    self.frame_pending = true;
                }
                // More than the one node: a frame round them, with its name.
                if fresh.len() > 1 {
                    let layouts = tidy::layouts(graph, exposed, self.show_previews);
                    if let Some(bounds) = fresh.iter().filter_map(|id| layouts.get(id).map(|l| l.rect)).reduce(|a, b| a.union(b)) {
                        let around = bounds.expand(24.0);
                        graph.frames.push(tequila_materials::graph::Frame {
                            title: name,
                            min: GraphVec2 { x: around.min.x, y: around.min.y - tidy::FRAME_TITLE },
                            max: GraphVec2 { x: around.max.x, y: around.max.y },
                        });
                    }
                }
                self.pending_textures = textures;
                self.selected = fresh.into_iter().collect();
                self.active = Some(shader);
                self.selected_frame = None;
                true
            }
        }
    }

    // ---- coordinates -------------------------------------------------------------------

    fn to_screen(&self, rect: egui::Rect, p: egui::Pos2) -> egui::Pos2 {
        rect.min + (p.to_vec2() + self.pan) * self.zoom
    }

    fn rect_to_screen(&self, rect: egui::Rect, r: egui::Rect) -> egui::Rect {
        egui::Rect::from_min_max(self.to_screen(rect, r.min), self.to_screen(rect, r.max))
    }

    fn to_canvas(&self, rect: egui::Rect, point: egui::Pos2) -> egui::Vec2 {
        (point - rect.min) / self.zoom - self.pan
    }

    // ---- the canvas ----------------------------------------------------------------------

    fn canvas(&mut self, ui: &mut egui::Ui, project: &Project, graph: &mut MaterialGraph, exposed: &[Param]) -> bool {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let ctx = ui.ctx().clone();
        let painter = ui.painter_at(rect);
        let pointer = ctx.input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos()));
        let over_map = self.minimap.is_some_and(|m| pointer.is_some_and(|p| m.rect.contains(p)));
        let inside = pointer.is_some_and(|p| rect.contains(p)) && response.contains_pointer() && !over_map;
        let mut committed = false;

        if std::mem::take(&mut self.frame_pending) {
            self.fit(rect, graph, exposed, None);
        }
        self.navigate(&ctx, rect, inside, pointer);

        // ---- layout ------------------------------------------------------------------
        let wired = view::connected(graph);
        let textures_doc = &project.document().textures;
        let texture_name = |id: tequila_materials::graph::TextureId| textures_doc.get(id).map(|s| s.display_name()).unwrap_or_else(|| "Missing picture".into());
        let mut order: Vec<NodeId> = graph.nodes().map(|(id, _)| id).collect();
        // Selected nodes drawn last, so on top; the active one above them.
        order.sort_by_key(|id| (self.selected.contains(id), self.active == Some(*id)));
        let layouts: HashMap<NodeId, NodeLayout> = order
            .iter()
            .filter_map(|id| graph.node(*id).map(|n| (*id, view::layout(n, *id, &wired, exposed, self.show_previews, &texture_name))))
            .collect();
        let z = self.zoom;
        let font = |size: f32| egui::FontId::proportional((size * z).clamp(5.0, 26.0));
        let detailed = z >= 0.45;

        // ---- grid --------------------------------------------------------------------
        self.grid(&painter, rect);

        // ---- frames ------------------------------------------------------------------
        let frame_rects: Vec<egui::Rect> = graph
            .frames
            .iter()
            .map(|f| egui::Rect::from_min_max(self.to_screen(rect, egui::pos2(f.min.x, f.min.y)), self.to_screen(rect, egui::pos2(f.max.x, f.max.y))))
            .collect();
        for (index, (frame, area)) in graph.frames.iter().zip(&frame_rects).enumerate() {
            let chosen = self.selected_frame == Some(index);
            painter.rect_filled(*area, 8.0 * z, egui::Color32::from_rgba_unmultiplied(120, 128, 150, 22));
            painter.rect_stroke(
                *area,
                8.0 * z,
                egui::Stroke::new(if chosen { 2.0_f32 } else { 1.0 }, if chosen { crate::theme::p().accent } else { egui::Color32::from_rgb(82, 88, 104) }),
            );
            let bar = egui::Rect::from_min_size(area.min, egui::vec2(area.width(), tidy::FRAME_TITLE * z));
            if detailed {
                painter.text(bar.left_center() + egui::vec2(10.0 * z, 0.0), egui::Align2::LEFT_CENTER, &frame.title, font(13.0), ink::text());
            }
            let corner = area.max;
            painter.line_segment([corner - egui::vec2(10.0, 3.0), corner - egui::vec2(3.0, 10.0)], (1.5_f32, egui::Color32::from_gray(130)));
        }

        // ---- wires -------------------------------------------------------------------
        let muted: HashSet<NodeId> = graph.nodes().filter(|(_, n)| n.muted).map(|(id, _)| id).collect();
        for link in graph.links() {
            let (Some(from), Some(to)) = (layouts.get(&link.from), layouts.get(&link.to)) else { continue };
            let (Some(a), Some(b)) = (from.output_at(link.from_socket), to.input_at(link.to_socket)) else { continue };
            let (a, b) = (self.to_screen(rect, a), self.to_screen(rect, b));
            if !rect.expand(200.0).contains(a) && !rect.expand(200.0).contains(b) {
                continue;
            }
            let ty = graph.node(link.from).and_then(|n| n.kind.output_type_at(link.from_socket)).unwrap_or(SocketType::Float);
            let lit = self.selected.contains(&link.from) || self.selected.contains(&link.to);
            let colour = if muted.contains(&link.from) {
                ink::MUTED.gamma_multiply(0.8)
            } else if self.insert_into == Some(*link) {
                ink::active()
            } else {
                view::type_colour(ty).gamma_multiply(if lit { 1.0 } else { 0.75 })
            };
            wire(&painter, a, b, colour, z, lit || self.insert_into == Some(*link));
        }

        // ---- nodes -------------------------------------------------------------------
        let shown_graph_textures = project.scene().textures.clone();
        let extent = {
            let b = project.scene().bounds();
            let size = b.max - b.min;
            size.x.max(size.y).max(size.z).clamp(1e-3, 1e3)
        };
        let hashes = if self.show_previews { previews::upstream_hashes(graph) } else { HashMap::new() };
        let mut shared_graph: Option<Arc<MaterialGraph>> = None;
        let soloed = project.solo().map(|(_, node, output)| (node, output));
        let hover_target = pointer.filter(|_| inside).and_then(|p| self.field_at(&layouts, &order, graph, rect, p));
        for id in &order {
            let Some(node) = graph.node(*id) else { continue };
            let layout = &layouts[id];
            let screen = self.rect_to_screen(rect, layout.rect);
            if !rect.intersects(screen.expand(20.0)) {
                continue;
            }
            let selected = self.selected.contains(id);
            let active = self.active == Some(*id);
            if let NodeKind::Reroute { ty } = node.kind {
                painter.circle_filled(screen.center(), (view::REROUTE * 0.5 * z).max(3.0), view::type_colour(ty));
                if selected {
                    painter.circle_stroke(screen.center(), (view::REROUTE * 0.5 * z).max(3.0) + 2.0, egui::Stroke::new(1.5_f32, crate::theme::p().accent));
                }
                continue;
            }
            let round = 6.0 * z;
            // A soft shadow: depth without a border.
            painter.rect_filled(screen.translate(egui::vec2(0.0, 3.0 * z)).expand(1.0 * z), round + 2.0, ink::shade(70));
            painter.rect_filled(screen, round, if node.muted { ink::body().gamma_multiply(0.75) } else { ink::body() });
            let header = self.rect_to_screen(rect, layout.header());
            let tint = if soloed.is_some_and(|(n, _)| n == *id) {
                egui::Color32::from_rgb(170, 140, 40)
            } else if node.muted {
                egui::Color32::from_gray(64)
            } else {
                view::family(&node.kind).tint()
            };
            let header_round = if layout.collapsed {
                egui::Rounding::same(round)
            } else {
                egui::Rounding { nw: round, ne: round, sw: 0.0, se: 0.0 }
            };
            painter.rect_filled(header, header_round, tint);
            if detailed {
                // The collapse arrow, then the title.
                let arrow = header.left_center() + egui::vec2(10.0 * z, 0.0);
                triangle(&painter, arrow, 4.0 * z, !layout.collapsed, egui::Color32::from_gray(200));
                let title_colour = if node.muted { egui::Color32::from_gray(170) } else { ink::header_text() };
                let clip = painter.with_clip_rect(header.shrink2(egui::vec2(4.0 * z, 0.0)).intersect(rect));
                clip.text(header.left_center() + egui::vec2(20.0 * z, 0.0), egui::Align2::LEFT_CENTER, view::title(node), font(12.5), title_colour);
                if soloed.is_some_and(|(n, _)| n == *id) {
                    clip.text(header.right_center() - egui::vec2(8.0 * z, 0.0), egui::Align2::RIGHT_CENTER, "SOLO", font(10.0), egui::Color32::BLACK);
                } else if node.muted {
                    clip.text(header.right_center() - egui::vec2(8.0 * z, 0.0), egui::Align2::RIGHT_CENTER, "MUTED", font(10.0), egui::Color32::from_gray(170));
                }
            }
            if selected || active {
                let colour = if active { ink::active().gamma_multiply(0.9) } else { crate::theme::p().accent };
                painter.rect_stroke(screen.expand(1.5), round + 1.5, egui::Stroke::new(if active { 2.0_f32 } else { 1.5 }, if active && selected { crate::theme::p().accent } else { colour }));
            }
            // The rows.
            if detailed && !layout.collapsed {
                for row in &layout.rows {
                    let area = self.rect_to_screen(rect, layout.row_rect(row));
                    if !rect.intersects(area) {
                        continue;
                    }
                    self.draw_row(&painter, &ctx, rect, graph, *id, node, layout, row, area, z, &wired, hover_target);
                    if row.kind == RowKind::Preview {
                        let output = previews::preview_output(&node.kind);
                        let hash = hashes.get(id).copied().unwrap_or(0) ^ (output as u64).rotate_left(17) ^ extent.to_bits() as u64;
                        let image = area.shrink2(egui::vec2(0.0, 1.0 * z));
                        painter.rect_filled(image, 3.0 * z, egui::Color32::from_gray(22));
                        if let Some(texture) = self.previews.get(&ctx, (*id, output), hash, &mut shared_graph, graph, &shown_graph_textures, extent) {
                            painter.image(texture.id(), image, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                        }
                        if self.previews.pending((*id, output), hash) {
                            painter.circle_filled(image.right_top() + egui::vec2(-6.0, 6.0) * z, 2.5 * z, egui::Color32::from_white_alpha(160));
                        }
                    }
                }
            }
            // Muted: the wire passes through, drawn as Blender draws it.
            if node.muted && !layout.collapsed {
                for (index, ty, out_at) in &layout.outputs {
                    let pass = graph
                        .links()
                        .iter()
                        .filter(|l| l.to == *id)
                        .find(|l| graph.node(l.from).and_then(|n| n.kind.output_type_at(l.from_socket)).is_some_and(|t| t.converts_to(*ty)));
                    if let Some(pass) = pass {
                        if let Some(in_at) = layout.input_at(pass.to_socket) {
                            wire(&painter, self.to_screen(rect, in_at), self.to_screen(rect, *out_at), ink::MUTED.gamma_multiply(0.7), z, false);
                        }
                    }
                    let _ = index;
                }
            }
            // Sockets.
            for (index, ty, at) in &layout.outputs {
                let _ = index;
                socket(&painter, self.to_screen(rect, *at), *ty, z, true);
            }
            for (socket_id, _, ty, at) in &layout.inputs {
                socket(&painter, self.to_screen(rect, *at), *ty, z, wired.contains(&(*id, *socket_id)));
            }
        }

        // ---- minimap ------------------------------------------------------------------
        self.minimap_ui(ui, &painter, rect, graph, &layouts);

        // ---- interaction --------------------------------------------------------------
        committed |= self.interact(&ctx, &painter, rect, inside, pointer, graph, &layouts, &order, &frame_rects, exposed);

        // ---- overlays ----------------------------------------------------------------
        committed |= self.typing_ui(ui, graph, z);
        committed |= self.dropdown_ui(&ctx, graph);
        committed |= self.context_ui(&ctx, rect, project, graph, exposed);
        if self.menu.is_some() {
            committed |= self.menu_ui(&ctx, rect, project, graph, &layouts);
        }
        self.canvas_rect = rect;
        committed
    }

    /// The overview in the canvas's corner: every node as a block in its
    /// family's colour, the part of the graph in view as a frame. Shown when
    /// the graph does not fit the view; a click or a drag on it moves the
    /// view there.
    fn minimap_ui(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: egui::Rect, graph: &MaterialGraph, layouts: &HashMap<NodeId, NodeLayout>) {
        self.minimap = None;
        let content = layouts.values().map(|l| l.rect).reduce(|a, b| a.union(b));
        let view = egui::Rect::from_min_max(self.to_canvas(rect, rect.min).to_pos2(), self.to_canvas(rect, rect.max).to_pos2());
        let Some(content) = content.filter(|c| self.show_map && !view.contains_rect(*c)) else {
            self.map_world = None;
            return;
        };
        if rect.width() < 420.0 || rect.height() < 300.0 {
            return;
        }
        let size = egui::vec2(190.0, 128.0);
        let area = egui::Rect::from_min_size(rect.max - size - egui::vec2(12.0, 12.0), size);
        let world = self.map_world.unwrap_or_else(|| content.union(view).expand(40.0));
        let inner = area.shrink(8.0);
        let scale = (inner.width() / world.width().max(1.0)).min(inner.height() / world.height().max(1.0));
        let map = Minimap { rect: area, offset: inner.center().to_vec2() - world.center().to_vec2() * scale, scale };

        painter.add(egui::epaint::Shadow { offset: egui::vec2(0.0, 3.0), blur: 14.0, spread: 0.0, color: ink::shade(80) }.as_shape(area, 10.0));
        painter.rect(area, 10.0, crate::theme::p().island, egui::Stroke::new(1.0_f32, crate::theme::p().island_line));
        let clip = painter.with_clip_rect(area.shrink(1.0));
        for (id, layout) in layouts {
            let Some(node) = graph.node(*id) else { continue };
            let block = egui::Rect::from_min_max(map.to_map(layout.rect.min), map.to_map(layout.rect.max));
            let tint = if node.muted { egui::Color32::from_gray(70) } else { view::family(&node.kind).tint() };
            clip.rect_filled(block, 1.5, tint);
            if self.selected.contains(id) {
                clip.rect_stroke(block.expand(1.0), 2.0, egui::Stroke::new(1.0_f32, crate::theme::p().accent));
            }
        }
        let seen = egui::Rect::from_min_max(map.to_map(view.min), map.to_map(view.max));
        clip.rect_filled(seen, 2.0, crate::theme::p().text.gamma_multiply(0.06));
        clip.rect_stroke(seen, 2.0, egui::Stroke::new(1.2_f32, crate::theme::p().icon));

        let response = ui.interact(area, ui.id().with("graph-minimap"), egui::Sense::click_and_drag());
        if response.drag_started() {
            self.map_world = Some(world);
        }
        if response.drag_stopped() {
            self.map_world = None;
        }
        if response.clicked() || response.dragged() {
            if let Some(at) = response.interact_pointer_pos() {
                let target = map.to_canvas(at);
                self.pan = rect.size() * 0.5 / self.zoom - target.to_vec2();
            }
        }
        response.on_hover_text("The whole graph: click or drag to look there");
        self.minimap = Some(map);
    }

    /// Zoom and pan from the wheel, the trackpad and a pinch.
    fn navigate(&mut self, ctx: &egui::Context, rect: egui::Rect, inside: bool, pointer: Option<egui::Pos2>) {
        if !inside || self.menu.is_some() {
            return;
        }
        let Some(at) = pointer else { return };
        let mut zoom = 1.0f32;
        let mut pan = egui::Vec2::ZERO;
        ctx.input(|i| {
            for event in &i.raw.events {
                match event {
                    // A mouse wheel zooms, as in Blender.
                    egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page, delta, .. } => {
                        zoom *= (delta.y * 0.12).exp();
                    }
                    // A trackpad's two fingers pan; with Ctrl they zoom.
                    egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, modifiers } => {
                        if modifiers.ctrl || modifiers.command {
                            zoom *= (delta.y * 0.004).exp();
                        } else {
                            pan += *delta;
                        }
                    }
                    egui::Event::Zoom(factor) => zoom *= factor,
                    _ => {}
                }
            }
        });
        if zoom != 1.0 {
            let before = self.to_canvas(rect, at);
            self.zoom = (self.zoom * zoom).clamp(0.15, 3.0);
            let after = self.to_canvas(rect, at);
            self.pan += after - before;
        }
        if pan != egui::Vec2::ZERO {
            self.pan += pan / self.zoom;
        }
    }

    fn grid(&self, painter: &egui::Painter, rect: egui::Rect) {
        let spacing = 24.0 * self.zoom;
        if spacing < 7.0 {
            return;
        }
        let offset = egui::vec2((self.pan.x * self.zoom).rem_euclid(spacing), (self.pan.y * self.zoom).rem_euclid(spacing));
        // Every fifth dot heavier, so a long way across still has landmarks.
        let first = ((-self.pan.x * self.zoom + offset.x) / spacing).round() as i64;
        let first_y = ((-self.pan.y * self.zoom + offset.y) / spacing).round() as i64;
        let radius = (1.1 * self.zoom).clamp(0.7, 1.6);
        let mut x = rect.left() + offset.x;
        let mut i = first;
        while x < rect.right() {
            let mut y = rect.top() + offset.y;
            let mut j = first_y;
            while y < rect.bottom() {
                let major = i.rem_euclid(5) == 0 && j.rem_euclid(5) == 0;
                painter.circle_filled(egui::pos2(x, y), if major { radius * 1.4 } else { radius }, if major { ink::dot_major() } else { ink::dot() });
                y += spacing;
                j += 1;
            }
            x += spacing;
            i += 1;
        }
    }

    /// Fit the view to everything, or to `only`.
    fn fit(&mut self, rect: egui::Rect, graph: &MaterialGraph, exposed: &[Param], only: Option<&BTreeSet<NodeId>>) {
        let layouts = tidy::layouts(graph, exposed, self.show_previews);
        let bounds = match only {
            Some(ids) if !ids.is_empty() => ids.iter().filter_map(|id| layouts.get(id).map(|l| l.rect)).reduce(|a, b| a.union(b)),
            _ => tidy::content_bounds(graph, exposed, self.show_previews),
        };
        if let Some(bounds) = bounds {
            let margin = 80.0;
            self.zoom = (rect.width() / (bounds.width() + margin)).min(rect.height() / (bounds.height() + margin)).clamp(0.25, 1.2);
            self.pan = rect.size() * 0.5 / self.zoom - bounds.center().to_vec2();
        }
    }

    /// The field under `point`, if any, for hover and for clicks.
    fn field_at(&self, layouts: &HashMap<NodeId, NodeLayout>, order: &[NodeId], graph: &MaterialGraph, rect: egui::Rect, point: egui::Pos2) -> Option<(Target, egui::Rect)> {
        if self.zoom < 0.45 {
            return None;
        }
        for id in order.iter().rev() {
            let layout = layouts.get(id)?;
            let screen = self.rect_to_screen(rect, layout.rect);
            if !screen.contains(point) {
                continue;
            }
            if layout.collapsed {
                return None;
            }
            let node = graph.node(*id)?;
            let wired = view::connected(graph);
            for row in &layout.rows {
                let area = self.rect_to_screen(rect, layout.row_rect(row));
                if !area.contains(point) {
                    continue;
                }
                let target = match row.kind {
                    RowKind::Input(index) => {
                        let (socket, ty, _) = *node.kind.inputs().get(index)?;
                        if wired.contains(&(*id, socket)) {
                            return None;
                        }
                        match view::inline_of(node, index, socket, ty) {
                            Inline::Number { .. } | Inline::Color | Inline::Bool => Target::Input { node: *id, index, component: None },
                            _ => return None,
                        }
                    }
                    RowKind::Component(index, axis) => Target::Input { node: *id, index, component: Some(axis) },
                    RowKind::Setting(setting) => Target::Setting { node: *id, setting },
                    RowKind::Value => match node.kind {
                        NodeKind::Constant(Value::Vector(_)) => {
                            let third = (area.height() / 3.0).max(1.0);
                            Target::Value { node: *id, component: Some((((point.y - area.top()) / third) as u8).min(2)) }
                        }
                        _ => Target::Value { node: *id, component: None },
                    },
                    _ => return None,
                };
                return Some((target, area));
            }
            return None;
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        &self,
        painter: &egui::Painter,
        ctx: &egui::Context,
        rect: egui::Rect,
        graph: &MaterialGraph,
        id: NodeId,
        node: &Node,
        layout: &NodeLayout,
        row: &view::Row,
        area: egui::Rect,
        z: f32,
        wired: &HashSet<(NodeId, Socket)>,
        hover: Option<(Target, egui::Rect)>,
    ) {
        let font = |size: f32| egui::FontId::proportional((size * z).clamp(5.0, 26.0));
        let clip = painter.with_clip_rect(area.expand(1.0).intersect(rect));
        let hovered = |target: Target| hover.is_some_and(|(t, _)| t == target);
        let typing = |target: Target| self.typing.as_ref().is_some_and(|t| t.target == target);
        match &row.kind {
            RowKind::Output(index) => {
                let name = node.kind.outputs().get(*index as usize).map(|o| o.0).unwrap_or("");
                clip.text(area.right_center(), egui::Align2::RIGHT_CENTER, name, font(11.5), ink::label());
            }
            RowKind::Preview => {}
            RowKind::Ramp => {
                if let NodeKind::Gradient { gradient } = &node.kind {
                    let bar = area.shrink2(egui::vec2(0.0, 3.0 * z));
                    let steps = (bar.width() / 2.0).max(2.0) as usize;
                    let mut mesh = egui::Mesh::default();
                    for i in 0..=steps {
                        let t = i as f32 / steps as f32;
                        let c = tequila_color::DisplayTransform::Srgb.apply_u8(gradient.evaluate(t));
                        let colour = egui::Color32::from_rgb(c[0], c[1], c[2]);
                        let x = bar.left() + t * bar.width();
                        mesh.colored_vertex(egui::pos2(x, bar.top()), colour);
                        mesh.colored_vertex(egui::pos2(x, bar.bottom()), colour);
                        if i > 0 {
                            let k = (i as u32) * 2;
                            mesh.add_triangle(k - 2, k - 1, k);
                            mesh.add_triangle(k - 1, k + 1, k);
                        }
                    }
                    clip.add(egui::Shape::mesh(mesh));
                    clip.rect_stroke(bar, 2.0 * z, egui::Stroke::new(1.0_f32, egui::Color32::from_gray(20)));
                    for stop in gradient.stops() {
                        let x = bar.left() + stop.at * bar.width();
                        let c = tequila_color::DisplayTransform::Srgb.apply_u8(stop.color);
                        let marker = [egui::pos2(x, bar.bottom() - 4.0 * z), egui::pos2(x - 4.0 * z, bar.bottom() + 3.0 * z), egui::pos2(x + 4.0 * z, bar.bottom() + 3.0 * z)];
                        clip.add(egui::Shape::convex_polygon(marker.to_vec(), egui::Color32::from_rgb(c[0], c[1], c[2]), egui::Stroke::new(1.0_f32, egui::Color32::from_gray(230))));
                    }
                }
            }
            RowKind::Note(text) => {
                clip.text(area.left_center(), egui::Align2::LEFT_CENTER, text, font(11.0), crate::theme::dim());
            }
            RowKind::Setting(setting) => {
                let target = Target::Setting { node: id, setting: *setting };
                match view::setting_field(node, *setting) {
                    Some(SettingField::Dropdown { current }) => {
                        let open = self.dropdown.as_ref().is_some_and(|d| d.node == id && d.setting == *setting);
                        clip.rect_filled(area, 3.0 * z, if hovered(target) || open { ink::field_hover() } else { ink::field() });
                        clip.text(area.left_center() + egui::vec2(6.0 * z, 0.0), egui::Align2::LEFT_CENTER, current, font(11.5), ink::text());
                        triangle(&clip, area.right_center() - egui::vec2(8.0 * z, 0.0), 3.5 * z, true, ink::label());
                    }
                    Some(SettingField::Number { label, int, range }) => {
                        let value = view::read(graph, target).map(|v| v.as_float()).unwrap_or(0.0);
                        number_field(&clip, area, z, label, value, int, range, hovered(target), typing(target));
                    }
                    Some(SettingField::Toggle { label, on }) => toggle(&clip, area, z, label, on, hovered(target)),
                    None => {}
                }
            }
            RowKind::Input(index) => {
                let Some((socket, ty, label)) = node.kind.inputs().get(*index).copied() else { return };
                let target = Target::Input { node: id, index: *index, component: None };
                if wired.contains(&(id, socket)) {
                    clip.text(area.left_center(), egui::Align2::LEFT_CENTER, label, font(11.5), ink::label());
                    return;
                }
                match view::inline_of(node, *index, socket, ty) {
                    Inline::Label | Inline::Vector => {
                        clip.text(area.left_center(), egui::Align2::LEFT_CENTER, label, font(11.5), ink::label());
                    }
                    Inline::Number { range } => {
                        let value = view::read(graph, target).map(|v| v.as_float()).unwrap_or(0.0);
                        number_field(&clip, area, z, label, value, false, range, hovered(target), typing(target));
                    }
                    Inline::Bool => {
                        let on = matches!(view::read(graph, target), Some(Value::Bool(true)));
                        toggle(&clip, area, z, label, on, hovered(target));
                    }
                    Inline::Color => {
                        let colour = view::read(graph, target).map(|v| v.as_color()).unwrap_or_default();
                        colour_field(&clip, area, z, label, colour, hovered(target));
                    }
                }
            }
            RowKind::Component(index, axis) => {
                let target = Target::Input { node: id, index: *index, component: Some(*axis) };
                let value = view::read(graph, target).map(|v| v.as_float()).unwrap_or(0.0);
                let label = ["X", "Y", "Z"][*axis as usize];
                // Stacked, joined, as Blender draws a vector.
                let area = area.expand2(egui::vec2(0.0, 1.0 * z));
                number_field(&clip, area, z, label, value, false, None, hovered(target), typing(target));
            }
            RowKind::Value => match node.kind {
                NodeKind::Constant(Value::Color(c)) => colour_field(&clip, area, z, "Color", c, hovered(Target::Value { node: id, component: None })),
                NodeKind::Constant(Value::Float(v)) => {
                    let target = Target::Value { node: id, component: None };
                    number_field(&clip, area, z, "Value", v, false, None, hovered(target), typing(target));
                }
                NodeKind::Constant(Value::Vector(v)) => {
                    let third = area.height() / 3.0;
                    for (axis, value) in [v.x, v.y, v.z].into_iter().enumerate() {
                        let target = Target::Value { node: id, component: Some(axis as u8) };
                        let part = egui::Rect::from_min_size(area.min + egui::vec2(0.0, third * axis as f32), egui::vec2(area.width(), third - 1.0));
                        number_field(&clip, part, z, ["X", "Y", "Z"][axis], value, false, None, hovered(target), typing(target));
                    }
                }
                NodeKind::Constant(Value::Bool(b)) => toggle(&clip, area, z, "Value", b, false),
                _ => {}
            },
        }
        let _ = (ctx, layout);
    }

    #[allow(clippy::too_many_arguments)]
    fn interact(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        rect: egui::Rect,
        inside: bool,
        pointer: Option<egui::Pos2>,
        graph: &mut MaterialGraph,
        layouts: &HashMap<NodeId, NodeLayout>,
        order: &[NodeId],
        frame_rects: &[egui::Rect],
        exposed: &[Param],
    ) -> bool {
        let mut committed = false;
        let z = self.zoom;
        let layer = painter.layer_id();
        let (pressed, released, secondary_pressed, secondary_released, middle_down, delta, modifiers) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.button_pressed(egui::PointerButton::Secondary),
                i.pointer.button_released(egui::PointerButton::Secondary),
                i.pointer.middle_down(),
                i.pointer.delta(),
                i.modifiers,
            )
        });
        // Something drawn over the canvas - a popup, a tooltip, another
        // window - has the pointer, not the canvas. (Every point of the
        // editor is "over an area": the editor is one.)
        let covered = pointer.and_then(|p| ctx.layer_id_at(p)).is_some_and(|l| l != layer);
        let overlay_open = self.menu.is_some() || self.dropdown.is_some() || self.context.is_some() || covered;
        let typing_focus = ctx.wants_keyboard_input();

        let xf = Xf { origin: rect.min, pan: self.pan, zoom: self.zoom };
        let socket_at = |point: egui::Pos2| -> Option<End> {
            let reach = (9.0 * z).max(7.0);
            for id in order.iter().rev() {
                let layout = &layouts[id];
                for (index, _, at) in &layout.outputs {
                    if xf.point(*at).distance(point) < reach {
                        return Some(End::Output(*id, *index));
                    }
                }
                for (socket, _, _, at) in &layout.inputs {
                    if xf.point(*at).distance(point) < reach {
                        return Some(End::Input(*id, *socket));
                    }
                }
            }
            None
        };
        let node_at = |point: egui::Pos2| -> Option<NodeId> {
            order.iter().rev().find(|id| xf.rect(layouts[*id].rect).expand(2.0).contains(point)).copied()
        };

        // ---- G: the selection follows the pointer ----------------------------------
        if let Some(grab) = &self.grab {
            if let Some(point) = pointer {
                let offset = self.to_canvas(rect, point) - grab.start;
                for (id, from) in &grab.nodes {
                    if let Some(node) = graph.node_mut(*id) {
                        node.position = GraphVec2 { x: snap(from.x + offset.x), y: snap(from.y + offset.y) };
                    }
                }
            }
            let (confirm, cancel) = ctx.input(|i| (i.pointer.primary_pressed() || i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape) || i.pointer.button_pressed(egui::PointerButton::Secondary)));
            if cancel {
                if let Some(grab) = self.grab.take() {
                    for (id, from) in grab.nodes {
                        if let Some(node) = graph.node_mut(id) {
                            node.position = from;
                        }
                    }
                }
            } else if confirm {
                if let Some(grab) = self.grab.take() {
                    let moved: Vec<NodeId> = grab.nodes.iter().map(|(id, _)| *id).collect();
                    apart(graph, &moved);
                }
                committed = true;
            }
            return committed;
        }

        // ---- presses ------------------------------------------------------------------
        if inside && pressed && !overlay_open && self.drag.is_none() {
            if let Some(point) = pointer {
                if modifiers.alt {
                    self.drag = Some(Drag::Pan);
                } else if let Some(end) = socket_at(point) {
                    // Pulling a wire off a connected input picks it up from
                    // its source, as in every node editor.
                    let end = match end {
                        End::Input(node, socket) => match graph.links().iter().find(|l| l.to == node && l.to_socket == socket).copied() {
                            Some(link) => {
                                graph.disconnect(node, socket);
                                End::Output(link.from, link.from_socket)
                            }
                            None => end,
                        },
                        other => other,
                    };
                    self.drag = Some(Drag::Wire { from: end });
                } else if let Some((target, area)) = self.field_at(layouts, order, graph, rect, point) {
                    self.typing = None;
                    match self.field_kind(graph, target) {
                        FieldKind::Number { int, range } => {
                            let start = view::read(graph, target).map(|v| v.as_float()).unwrap_or(0.0);
                            self.drag = Some(Drag::Field { target, start, int, range, travel: 0.0, moved: false });
                        }
                        FieldKind::Toggle => {
                            let on = matches!(view::read(graph, target), Some(Value::Bool(true)));
                            if view::write(graph, target, Value::Bool(!on)) {
                                committed = true;
                            }
                        }
                        FieldKind::Colour => {
                            self.dropdown = None;
                            self.context = None;
                            self.typing = Some(Typing { target, text: String::new(), focus: false, rect: area, fresh: true });
                        }
                        FieldKind::Dropdown(setting) => {
                            self.dropdown = Some(Dropdown { node: target.node(), setting, at: area.left_bottom(), width: area.width(), fresh: true });
                        }
                    }
                    self.select_one(target.node(), modifiers.shift);
                } else if let Some(id) = node_at(point) {
                    let layout = &layouts[&id];
                    let header = self.rect_to_screen(rect, layout.header());
                    let arrow = egui::Rect::from_center_size(header.left_center() + egui::vec2(10.0 * z, 0.0), egui::vec2(16.0, 16.0) * z.max(0.6));
                    if arrow.contains(point) && !matches!(graph.node(id).map(|n| &n.kind), Some(NodeKind::Reroute { .. })) {
                        if let Some(node) = graph.node_mut(id) {
                            node.collapsed = !node.collapsed;
                            committed = true;
                        }
                    } else if modifiers.ctrl && modifiers.shift {
                        // Solo it, cycling through its outputs on each click.
                        let outputs = graph.node(id).map(|n| n.kind.outputs().len() as u8).unwrap_or(1).max(1);
                        let next = match self.solo_current() {
                            Some((n, o)) if n == id => (o + 1) % outputs,
                            _ => 0,
                        };
                        if id != graph.output() && graph.soloed_at(id, next).is_some() {
                            self.solo_request = Some(Some((id, next)));
                            self.solo_shown = Some((id, next));
                        }
                    } else {
                        if modifiers.shift {
                            if self.selected.contains(&id) && self.active == Some(id) {
                                self.selected.remove(&id);
                                self.active = None;
                            } else {
                                self.selected.insert(id);
                                self.active = Some(id);
                            }
                        } else if !self.selected.contains(&id) {
                            self.selected = BTreeSet::from([id]);
                            self.active = Some(id);
                        } else {
                            self.active = Some(id);
                        }
                        self.selected_frame = None;
                        let start = self.to_canvas(rect, point);
                        let nodes = self.selected.iter().filter_map(|id| graph.node(*id).map(|n| (*id, n.position))).collect();
                        self.drag = Some(Drag::Move { start, nodes, moved: false });
                    }
                } else if let Some(index) = frame_rects.iter().rposition(|r| egui::Rect::from_min_size(r.max - egui::vec2(14.0, 14.0), egui::vec2(14.0, 14.0)).contains(point)) {
                    self.clear_selection();
                    self.selected_frame = Some(index);
                    self.drag = Some(Drag::FrameResize { index });
                } else if let Some(index) = frame_rects.iter().rposition(|r| egui::Rect::from_min_size(r.min, egui::vec2(r.width(), tidy::FRAME_TITLE * z)).contains(point)) {
                    self.clear_selection();
                    self.selected_frame = Some(index);
                    let frame = &graph.frames[index];
                    let min = egui::vec2(frame.min.x, frame.min.y);
                    // What it carries: every node whose corner is inside.
                    self.frame_members = graph.nodes().filter(|(_, n)| frame.contains(n.position)).map(|(id, n)| (id, egui::vec2(n.position.x, n.position.y) - min)).collect();
                    self.drag = Some(Drag::Frame { index, grab: self.to_canvas(rect, point) - min });
                } else {
                    self.drag = Some(Drag::Box { start: point, add: modifiers.shift, remove: modifiers.ctrl });
                }
            }
        }
        if inside && middle_down && self.drag.is_none() && !overlay_open {
            self.drag = Some(Drag::Pan);
        }
        if inside && secondary_pressed && !overlay_open && self.drag.is_none() {
            if let Some(point) = pointer {
                if modifiers.ctrl {
                    self.drag = Some(Drag::Cut { points: vec![point] });
                }
            }
        }
        if inside && secondary_released && !overlay_open && !matches!(self.drag, Some(Drag::Cut { .. })) {
            if let Some(point) = pointer {
                let canvas = self.to_canvas(rect, point);
                match node_at(point) {
                    Some(id) => {
                        if !self.selected.contains(&id) {
                            self.selected = BTreeSet::from([id]);
                        }
                        self.active = Some(id);
                        self.context = Some(ContextMenu { node: Some(id), at: point, canvas });
                    }
                    None => self.menu = Some(menu::AddMenu::new(canvas, point, None)),
                }
            }
        }

        // ---- drags --------------------------------------------------------------------
        let primary_down = ctx.input(|i| i.pointer.primary_down());
        match self.drag.clone() {
            Some(Drag::Pan) => {
                self.pan += delta / self.zoom;
                if !ctx.input(|i| i.pointer.primary_down() || i.pointer.middle_down()) {
                    self.drag = None;
                }
            }
            Some(Drag::Move { start, nodes, moved }) => {
                if let Some(point) = pointer {
                    let offset = self.to_canvas(rect, point) - start;
                    let moving = moved || offset.length() * self.zoom > 3.0;
                    if moving {
                        for (id, from) in &nodes {
                            if let Some(node) = graph.node_mut(*id) {
                                // Snapped to the grid, so nodes line up without effort.
                                node.position = GraphVec2 { x: snap(from.x + offset.x), y: snap(from.y + offset.y) };
                            }
                        }
                        self.drag = Some(Drag::Move { start, nodes: nodes.clone(), moved: true });
                        // One node with no wires, held over a wire: it goes in between.
                        self.insert_into = if nodes.len() == 1 {
                            let id = nodes[0].0;
                            let free = !graph.links().iter().any(|l| l.from == id || l.to == id);
                            if free {
                                let tidy_layouts = tidy::layouts(graph, exposed, self.show_previews);
                                tidy_layouts.get(&id).and_then(|l| self.wire_under(graph, &tidy_layouts, rect, self.rect_to_screen(rect, l.rect), id))
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                    }
                }
                if released {
                    self.drag = None;
                    if let Some(link) = self.insert_into.take() {
                        if let Some(&(id, _)) = nodes.first() {
                            insert(graph, link, id);
                        }
                    }
                    let moved: Vec<NodeId> = nodes.iter().map(|(id, _)| *id).collect();
                    apart(graph, &moved);
                    committed = true;
                }
            }
            Some(Drag::Frame { index, grab }) => {
                if let (Some(point), true) = (pointer, index < graph.frames.len()) {
                    let at = self.to_canvas(rect, point) - grab;
                    let at = egui::vec2(snap(at.x), snap(at.y));
                    let frame = &mut graph.frames[index];
                    let size = egui::vec2(frame.max.x - frame.min.x, frame.max.y - frame.min.y);
                    frame.min = GraphVec2 { x: at.x, y: at.y };
                    frame.max = GraphVec2 { x: at.x + size.x, y: at.y + size.y };
                    for (id, offset) in &self.frame_members {
                        if let Some(node) = graph.node_mut(*id) {
                            node.position = GraphVec2 { x: at.x + offset.x, y: at.y + offset.y };
                        }
                    }
                }
                if released {
                    self.drag = None;
                    committed = true;
                }
            }
            Some(Drag::FrameResize { index }) => {
                if let (Some(point), true) = (pointer, index < graph.frames.len()) {
                    let at = self.to_canvas(rect, point);
                    let frame = &mut graph.frames[index];
                    frame.max = GraphVec2 { x: at.x.max(frame.min.x + 120.0), y: at.y.max(frame.min.y + tidy::FRAME_TITLE + 60.0) };
                }
                if released {
                    self.drag = None;
                    committed = true;
                }
            }
            Some(Drag::Box { start, add, remove }) => {
                if let Some(point) = pointer {
                    let area = egui::Rect::from_two_pos(start, point);
                    if area.width() + area.height() > 4.0 {
                        painter.rect_filled(area, 2.0, crate::theme::p().accent.gamma_multiply(0.08));
                        painter.rect_stroke(area, 2.0, egui::Stroke::new(1.0_f32, crate::theme::p().accent.gamma_multiply(0.7)));
                    }
                    if released {
                        if area.width() + area.height() <= 4.0 {
                            // A click on nothing: nothing selected.
                            if !add {
                                self.clear_selection();
                            }
                        } else {
                            let inside: Vec<NodeId> = order.iter().filter(|id| area.intersects(self.rect_to_screen(rect, layouts[*id].rect))).copied().collect();
                            if !add && !remove {
                                self.selected.clear();
                            }
                            for id in inside {
                                if remove {
                                    self.selected.remove(&id);
                                } else {
                                    self.selected.insert(id);
                                }
                            }
                            self.active = self.selected.iter().next_back().copied();
                            self.selected_frame = None;
                        }
                    }
                }
                if released || !primary_down {
                    self.drag = None;
                }
            }
            Some(Drag::Cut { mut points }) => {
                if let Some(point) = pointer {
                    if points.last().is_none_or(|l| l.distance(point) > 4.0) {
                        points.push(point);
                    }
                }
                for shape in egui::Shape::dashed_line(&points, egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(240, 110, 90)), 6.0, 4.0) {
                    painter.add(shape);
                }
                if ctx.input(|i| i.pointer.button_released(egui::PointerButton::Secondary)) {
                    let cut: Vec<Link> = graph.links().iter().copied().filter(|link| self.crosses(layouts, rect, link, &points)).collect();
                    for link in &cut {
                        graph.disconnect(link.to, link.to_socket);
                    }
                    if !cut.is_empty() {
                        committed = true;
                    }
                    self.drag = None;
                } else {
                    self.drag = Some(Drag::Cut { points });
                }
            }
            Some(Drag::Field { target, start, int, range, mut travel, mut moved }) => {
                travel += delta.x;
                if travel.abs() > 3.0 {
                    moved = true;
                }
                if moved {
                    let span = match range {
                        Some((lo, hi)) if hi > lo => (hi - lo) / 200.0,
                        _ if int => 0.05,
                        _ => (start.abs().max(1.0)) * 0.005,
                    };
                    let fine = if modifiers.shift { 0.1 } else { 1.0 };
                    let mut value = start + travel * span * fine;
                    if modifiers.ctrl {
                        let step = match range {
                            Some((lo, hi)) if hi > lo => (hi - lo) / 20.0,
                            _ => 0.1,
                        };
                        value = (value / step).round() * step;
                    }
                    if let Some((lo, hi)) = range {
                        value = value.clamp(lo, hi);
                    }
                    if int {
                        value = value.round();
                    }
                    view::write(graph, target, Value::Float(value));
                }
                self.drag = Some(Drag::Field { target, start, int, range, travel, moved });
                if released {
                    self.drag = None;
                    if moved {
                        committed = true;
                    } else if let (Some((_, area)), Some(_)) = (pointer.and_then(|p| self.field_at(layouts, order, graph, rect, p)), pointer) {
                        // A click without a drag: type a value.
                        let text = match view::read(graph, target) {
                            Some(Value::Float(v)) if int => format!("{}", v.round() as i64),
                            Some(Value::Float(v)) => format!("{}", (v * 10000.0).round() / 10000.0),
                            _ => String::new(),
                        };
                        self.typing = Some(Typing { target, text, focus: true, rect: area, fresh: true });
                    }
                }
            }
            Some(Drag::Wire { from }) => {
                if let Some(point) = pointer {
                    let start = match from {
                        End::Output(id, index) => layouts.get(&id).and_then(|l| l.output_at(index)),
                        End::Input(id, socket) => layouts.get(&id).and_then(|l| l.input_at(socket)),
                    }
                    .map(|p| self.to_screen(rect, p));
                    if let Some(start) = start {
                        let target = socket_at(point);
                        let (ok, why) = match (from, target) {
                            (End::Output(a, out), Some(End::Input(b, socket))) | (End::Input(b, socket), Some(End::Output(a, out))) => compatible(graph, a, out, b, socket),
                            _ => (true, None),
                        };
                        let colour = if ok { ink::text() } else { egui::Color32::from_rgb(230, 80, 70) };
                        let (a, b) = if matches!(from, End::Output(..)) { (start, point) } else { (point, start) };
                        wire(painter, a, b, colour, z, true);
                        if let Some(why) = &why {
                            painter.text(point + egui::vec2(14.0, 14.0), egui::Align2::LEFT_TOP, why, egui::FontId::proportional(12.0), egui::Color32::from_rgb(240, 120, 110));
                        }
                        if released {
                            self.drag = None;
                            match (from, target) {
                                (End::Output(a, out), Some(End::Input(b, socket))) | (End::Input(b, socket), Some(End::Output(a, out))) => {
                                    if ok {
                                        adopt(graph, a, out, b);
                                        graph.connect_output(a, out, b, socket);
                                        committed = true;
                                    } else {
                                        self.message = why;
                                    }
                                }
                                (_, None) => {
                                    // Dropped in empty space: offer what could
                                    // go there, and wire it in on choosing.
                                    self.menu = Some(menu::AddMenu::new(self.to_canvas(rect, point), point, Some(from)));
                                }
                                _ => {}
                            }
                        }
                    }
                }
                if released {
                    self.drag = None;
                }
            }
            None => {}
        }

        // ---- keys ---------------------------------------------------------------------
        if inside && !typing_focus && self.menu.is_none() {
            committed |= self.keys(ctx, rect, pointer, graph, exposed, layouts);
        }
        committed
    }

    fn solo_current(&self) -> Option<(NodeId, u8)> {
        self.solo_shown
    }

    fn clear_selection(&mut self) {
        self.selected.clear();
        self.active = None;
        self.selected_frame = None;
    }

    fn select_one(&mut self, id: NodeId, add: bool) {
        if !add && !self.selected.contains(&id) {
            self.selected.clear();
        }
        self.selected.insert(id);
        self.active = Some(id);
        self.selected_frame = None;
    }

    /// The keys, as Blender's node editor has them.
    fn keys(&mut self, ctx: &egui::Context, rect: egui::Rect, pointer: Option<egui::Pos2>, graph: &mut MaterialGraph, exposed: &[Param], _layouts: &HashMap<NodeId, NodeLayout>) -> bool {
        let mut committed = false;
        let (m, pressed) = ctx.input(|i| (i.modifiers, i.keys_down.clone()));
        let _ = pressed;
        let key = |k: egui::Key| ctx.input(|i| i.key_pressed(k));
        let plain = !m.ctrl && !m.command && !m.alt && !m.shift;
        let at_pointer = pointer.map(|p| (self.to_canvas(rect, p), p));
        let output = graph.output();

        if (m.shift && key(egui::Key::A) && !m.ctrl) || (plain && key(egui::Key::Tab)) {
            if let Some((canvas, screen)) = at_pointer {
                self.menu = Some(menu::AddMenu::new(canvas, screen, None));
            }
        }
        if plain && key(egui::Key::A) {
            self.selected = graph.nodes().map(|(id, _)| id).collect();
        }
        if m.alt && key(egui::Key::A) {
            self.clear_selection();
        }
        if (m.ctrl || m.command) && key(egui::Key::I) {
            let all: BTreeSet<NodeId> = graph.nodes().map(|(id, _)| id).collect();
            self.selected = all.difference(&self.selected).copied().collect();
        }
        let removable: Vec<NodeId> = self.selected.iter().copied().filter(|id| *id != output).collect();
        if (plain && (key(egui::Key::X) || key(egui::Key::Delete) || key(egui::Key::Backspace))) && !removable.is_empty() {
            for id in &removable {
                graph.remove_node(*id);
            }
            self.clear_selection();
            committed = true;
        } else if plain && (key(egui::Key::Delete) || key(egui::Key::Backspace) || key(egui::Key::X)) {
            if let Some(index) = self.selected_frame.filter(|&i| i < graph.frames.len()) {
                // The frame goes; what was in it stays.
                graph.frames.remove(index);
                self.selected_frame = None;
                committed = true;
            }
        }
        if (m.ctrl || m.command) && !m.shift && key(egui::Key::X) && !removable.is_empty() {
            for id in &removable {
                dissolve(graph, *id);
            }
            self.clear_selection();
            committed = true;
        }
        if plain && key(egui::Key::M) && !removable.is_empty() {
            let mute = removable.iter().any(|id| graph.node(*id).is_some_and(|n| !n.muted));
            for id in &removable {
                if let Some(node) = graph.node_mut(*id) {
                    node.muted = mute;
                }
            }
            committed = true;
        }
        if plain && key(egui::Key::H) && !self.selected.is_empty() {
            let collapse = self.selected.iter().any(|id| graph.node(*id).is_some_and(|n| !n.collapsed));
            for id in self.selected.clone() {
                if let Some(node) = graph.node_mut(id) {
                    node.collapsed = collapse;
                }
            }
            committed = true;
        }
        if plain && key(egui::Key::G) && !self.selected.is_empty() {
            if let Some((canvas, _)) = at_pointer {
                self.grab = Some(Grab { start: canvas, nodes: self.selected.iter().filter_map(|id| graph.node(*id).map(|n| (*id, n.position))).collect() });
            }
        }
        if (m.ctrl || m.command) && key(egui::Key::C) && !removable.is_empty() {
            self.copy(graph, &removable);
        }
        if (m.ctrl || m.command) && key(egui::Key::V) {
            if let Some((canvas, _)) = at_pointer {
                committed |= self.paste(graph, canvas);
            }
        }
        if m.shift && !m.ctrl && key(egui::Key::D) && !removable.is_empty() {
            self.copy(graph, &removable);
            if let Some((canvas, _)) = at_pointer {
                // Pasted where they were, then picked up to be placed.
                let centre = self.clipboard_centre().unwrap_or(canvas);
                if self.paste(graph, centre + egui::vec2(24.0, 24.0)) {
                    self.grab = Some(Grab { start: canvas, nodes: self.selected.iter().filter_map(|id| graph.node(*id).map(|n| (*id, n.position))).collect() });
                }
            }
        }
        if (m.ctrl || m.command) && key(egui::Key::J) && !self.selected.is_empty() {
            let layouts = tidy::layouts(graph, exposed, self.show_previews);
            if let Some(bounds) = self.selected.iter().filter_map(|id| layouts.get(id).map(|l| l.rect)).reduce(|a, b| a.union(b)) {
                let around = bounds.expand(24.0);
                graph.frames.push(tequila_materials::graph::Frame {
                    title: "Frame".into(),
                    min: GraphVec2 { x: around.min.x, y: around.min.y - tidy::FRAME_TITLE },
                    max: GraphVec2 { x: around.max.x, y: around.max.y },
                });
                self.selected_frame = Some(graph.frames.len() - 1);
                self.selected.clear();
                self.active = None;
                committed = true;
            }
        }
        if plain && key(egui::Key::F2) && self.active.is_some() {
            self.focus_label = true;
        }
        if plain && key(egui::Key::Home) {
            self.fit(rect, graph, exposed, None);
        }
        // The side panels, as Blender's T and N.
        if plain && key(egui::Key::T) {
            self.show_browser = !self.show_browser;
        }
        if plain && key(egui::Key::N) {
            self.show_settings = !self.show_settings;
        }
        if plain && key(egui::Key::Period) {
            let selected = self.selected.clone();
            self.fit(rect, graph, exposed, Some(&selected));
        }
        if plain && key(egui::Key::Escape) {
            self.clear_selection();
        }
        committed
    }

    fn copy(&mut self, graph: &MaterialGraph, ids: &[NodeId]) {
        let set: HashSet<NodeId> = ids.iter().copied().collect();
        self.clipboard = Some(Clipboard {
            nodes: ids.iter().filter_map(|id| graph.node(*id).map(|n| (*id, n.clone()))).collect(),
            links: graph.links().iter().filter(|l| set.contains(&l.from) && set.contains(&l.to)).copied().collect(),
        });
    }

    fn clipboard_centre(&self) -> Option<egui::Vec2> {
        let clip = self.clipboard.as_ref()?;
        let n = clip.nodes.len().max(1) as f32;
        let sum = clip.nodes.iter().fold(egui::Vec2::ZERO, |s, (_, node)| s + egui::vec2(node.position.x, node.position.y));
        Some(sum / n)
    }

    /// Paste what was copied, centred on `at`, wired among itself.
    fn paste(&mut self, graph: &mut MaterialGraph, at: egui::Vec2) -> bool {
        let Some(clip) = self.clipboard.clone() else { return false };
        let Some(centre) = self.clipboard_centre() else { return false };
        let mut fresh: HashMap<NodeId, NodeId> = HashMap::new();
        for (old, node) in &clip.nodes {
            let mut node = node.clone();
            node.position = GraphVec2 { x: snap(node.position.x - centre.x + at.x), y: snap(node.position.y - centre.y + at.y) };
            fresh.insert(*old, graph.add_node(node));
        }
        for link in &clip.links {
            if let (Some(from), Some(to)) = (fresh.get(&link.from), fresh.get(&link.to)) {
                graph.connect_output(*from, link.from_socket, *to, link.to_socket);
            }
        }
        let pasted: Vec<NodeId> = fresh.values().copied().collect();
        apart(graph, &pasted);
        self.selected = fresh.values().copied().collect();
        self.active = fresh.values().next().copied();
        self.selected_frame = None;
        true
    }

    /// The wire a node at `screen` sits on, if any: one it could go into.
    fn wire_under(&self, graph: &MaterialGraph, layouts: &HashMap<NodeId, NodeLayout>, rect: egui::Rect, screen: egui::Rect, id: NodeId) -> Option<Link> {
        let kind = &graph.node(id)?.kind;
        self.wire_fitting(graph, layouts, rect, screen, kind, Some(id))
    }

    /// The wire under `screen` that a node of `kind` could go into: `own`
    /// is the node itself when it is in the graph already, whose wires are
    /// not the ones meant.
    fn wire_fitting(&self, graph: &MaterialGraph, layouts: &HashMap<NodeId, NodeLayout>, rect: egui::Rect, screen: egui::Rect, kind: &NodeKind, own: Option<NodeId>) -> Option<Link> {
        let inputs = kind.inputs();
        let outputs = kind.outputs();
        if inputs.is_empty() || outputs.is_empty() {
            return None;
        }
        graph.links().iter().copied().find(|link| {
            if own.is_some_and(|id| link.from == id || link.to == id) {
                return false;
            }
            let (Some(a), Some(b)) = (
                layouts.get(&link.from).and_then(|l| l.output_at(link.from_socket)),
                layouts.get(&link.to).and_then(|l| l.input_at(link.to_socket)),
            ) else {
                return false;
            };
            let points = bezier_points(self.to_screen(rect, a), self.to_screen(rect, b), self.zoom);
            if !points.iter().any(|p| screen.contains(*p)) {
                return false;
            }
            // It must be able to take the wire's value and give one back.
            let carried = graph.node(link.from).and_then(|n| n.kind.output_type_at(link.from_socket));
            let wanted = graph.node(link.to).and_then(|n| n.kind.inputs().into_iter().find(|i| i.0 == link.to_socket)).map(|i| i.1);
            matches!((carried, wanted), (Some(c), Some(w)) if inputs.iter().any(|i| c.converts_to(i.1)) && outputs.iter().any(|o| o.1.converts_to(w)))
        })
    }

    /// Whether a link's wire crosses the cut line.
    fn crosses(&self, layouts: &HashMap<NodeId, NodeLayout>, rect: egui::Rect, link: &Link, cut: &[egui::Pos2]) -> bool {
        let (Some(a), Some(b)) = (
            layouts.get(&link.from).and_then(|l| l.output_at(link.from_socket)),
            layouts.get(&link.to).and_then(|l| l.input_at(link.to_socket)),
        ) else {
            return false;
        };
        let curve = bezier_points(self.to_screen(rect, a), self.to_screen(rect, b), self.zoom);
        curve.windows(2).any(|w| cut.windows(2).any(|c| segments_cross(w[0], w[1], c[0], c[1])))
    }

    fn field_kind(&self, graph: &MaterialGraph, target: Target) -> FieldKind {
        let Some(node) = graph.node(target.node()) else { return FieldKind::Toggle };
        match target {
            Target::Setting { setting, .. } => match view::setting_field(node, setting) {
                Some(SettingField::Dropdown { .. }) => FieldKind::Dropdown(setting),
                Some(SettingField::Number { int, range, .. }) => FieldKind::Number { int, range },
                _ => FieldKind::Toggle,
            },
            Target::Input { index, component: Some(_), .. } => {
                let _ = index;
                FieldKind::Number { int: false, range: None }
            }
            Target::Input { index, component: None, .. } => {
                let Some((socket, ty, _)) = node.kind.inputs().get(index).copied() else { return FieldKind::Toggle };
                match view::inline_of(node, index, socket, ty) {
                    Inline::Number { range } => FieldKind::Number { int: false, range },
                    Inline::Color => FieldKind::Colour,
                    _ => FieldKind::Toggle,
                }
            }
            Target::Value { component, .. } => match node.kind {
                NodeKind::Constant(Value::Color(_)) => FieldKind::Colour,
                NodeKind::Constant(Value::Bool(_)) => FieldKind::Toggle,
                _ => {
                    let _ = component;
                    FieldKind::Number { int: false, range: None }
                }
            },
        }
    }

    /// A field being typed into, or a colour being picked.
    fn typing_ui(&mut self, ui: &mut egui::Ui, graph: &mut MaterialGraph, z: f32) -> bool {
        let Some(mut typing) = self.typing.take() else { return false };
        let node = graph.node(typing.target.node()).cloned();
        let Some(node) = node else { return false };
        let mut committed = false;
        if matches!(self.field_kind(graph, typing.target), FieldKind::Colour) {
            // egui's own picker, opened on the swatch.
            let mut colour = view::read(graph, typing.target).map(|v| v.as_color()).unwrap_or_default();
            let mut keep = true;
            egui::Area::new(egui::Id::new("graph-colour-picker"))
                .fixed_pos(typing.rect.left_bottom() + egui::vec2(0.0, 4.0))
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        let mut rgb = crate::color_picker::to_picker(colour);
                        let mut hsva = egui::ecolor::Hsva::from_rgb(rgb);
                        if egui::widgets::color_picker::color_picker_hsva_2d(ui, &mut hsva, egui::widgets::color_picker::Alpha::Opaque) {
                            rgb = hsva.to_rgb();
                            colour = crate::color_picker::from_picker(rgb);
                            view::write(graph, typing.target, Value::Color(colour));
                        }
                        if ui.button("Done").clicked() {
                            keep = false;
                            committed = true;
                        }
                    });
                });
            let outside = !std::mem::take(&mut typing.fresh) && ui.input(|i| i.pointer.any_pressed()) && !over_popup(ui.ctx(), "graph-colour-picker");
            if outside || ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter)) {
                keep = false;
                committed = true;
            }
            if keep {
                self.typing = Some(typing);
            }
            let _ = node;
            return committed;
        }
        let font = egui::FontId::proportional((11.5 * z).clamp(9.0, 22.0));
        let edit = egui::TextEdit::singleline(&mut typing.text).font(font).margin(egui::vec2(4.0, 1.0)).horizontal_align(egui::Align::Center);
        let response = ui.put(typing.rect, edit);
        if std::mem::take(&mut typing.focus) {
            response.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(typing.text.chars().count()));
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), response.id);
            }
        }
        let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if response.lost_focus() || cancel {
            if !cancel {
                let parsed = crate::numeric::parse(&typing.text, None).or_else(|| crate::numeric::parse(&format!("={}", typing.text), None));
                if let Some(v) = parsed {
                    if view::write(graph, typing.target, Value::Float(v as f32)) {
                        committed = true;
                    }
                }
            }
        } else {
            self.typing = Some(typing);
        }
        committed
    }

    fn dropdown_ui(&mut self, ctx: &egui::Context, graph: &mut MaterialGraph) -> bool {
        let Some(dropdown) = &mut self.dropdown else { return false };
        let fresh = std::mem::take(&mut dropdown.fresh);
        let Some(node) = graph.node(dropdown.node).cloned() else {
            self.dropdown = None;
            return false;
        };
        let Some((options, current)) = view::choices(&node, dropdown.setting) else {
            self.dropdown = None;
            return false;
        };
        let mut chosen = None;
        let mut close = false;
        egui::Area::new(egui::Id::new("graph-dropdown")).fixed_pos(dropdown.at + egui::vec2(0.0, 2.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4.0)).show(ui, |ui| {
                ui.set_min_width(dropdown.width.max(140.0));
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for (key, label) in &options {
                        if ui.selectable_label(*key == current, *label).clicked() {
                            chosen = Some(*key);
                        }
                    }
                });
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || (!fresh && ctx.input(|i| i.pointer.any_pressed()) && !over_popup(ctx, "graph-dropdown") && chosen.is_none()) {
            close = true;
        }
        let setting = dropdown.setting;
        let id = dropdown.node;
        if let Some(key) = chosen {
            self.dropdown = None;
            return view::choose(graph, id, setting, key);
        }
        if close {
            self.dropdown = None;
        }
        false
    }

    fn context_ui(&mut self, ctx: &egui::Context, rect: egui::Rect, project: &Project, graph: &mut MaterialGraph, exposed: &[Param]) -> bool {
        let Some(menu) = &self.context else { return false };
        let at = menu.at;
        let canvas = menu.canvas;
        let target = menu.node;
        let mut action: Option<&str> = None;
        egui::Area::new(egui::Id::new("graph-context")).fixed_pos(at).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4.0)).show(ui, |ui| {
                ui.set_min_width(190.0);
                let mut item = |ui: &mut egui::Ui, label: &str, keys: &str, name: &'static str| {
                    // The keys by this computer's names: these said Ctrl on a Mac.
                    let response = ui.add(egui::Button::new(label).shortcut_text(crate::theme::keys(keys)).frame(false).min_size(egui::vec2(180.0, 22.0)));
                    if response.clicked() {
                        action = Some(name);
                    }
                };
                if target.is_some() {
                    item(ui, "Duplicate", "Shift + D", "duplicate");
                    item(ui, "Copy", "Ctrl + C", "copy");
                    item(ui, "Mute", "M", "mute");
                    item(ui, "Collapse", "H", "collapse");
                    item(ui, "Solo output", "Ctrl + Shift + Click", "solo");
                    item(ui, "Frame selection", "Ctrl + J", "frame");
                    ui.separator();
                    item(ui, "Delete, keep wire", "Ctrl + X", "dissolve");
                    item(ui, "Delete", "X", "delete");
                } else {
                    item(ui, "Add…", "Shift + A", "add");
                    item(ui, "Paste", "Ctrl + V", "paste");
                }
                ui.separator();
                item(ui, "Tidy up", "", "tidy");
                item(ui, "Fit all", "Home", "fit");
            });
        });
        let close = ctx.input(|i| i.key_pressed(egui::Key::Escape)) || (ctx.input(|i| i.pointer.any_pressed()) && !over_popup(ctx, "graph-context") && action.is_none());
        let Some(action) = action else {
            if close {
                self.context = None;
            }
            return false;
        };
        self.context = None;
        let output = graph.output();
        let chosen: Vec<NodeId> = self.selected.iter().copied().filter(|id| *id != output).collect();
        let _ = project;
        match action {
            "duplicate" if !chosen.is_empty() => {
                self.copy(graph, &chosen);
                let centre = self.clipboard_centre().unwrap_or(canvas);
                self.paste(graph, centre + egui::vec2(24.0, 24.0));
                true
            }
            "copy" => {
                self.copy(graph, &chosen);
                false
            }
            "paste" => self.paste(graph, canvas),
            "mute" => {
                let mute = chosen.iter().any(|id| graph.node(*id).is_some_and(|n| !n.muted));
                for id in &chosen {
                    if let Some(n) = graph.node_mut(*id) {
                        n.muted = mute;
                    }
                }
                true
            }
            "collapse" => {
                for id in self.selected.clone() {
                    if let Some(n) = graph.node_mut(id) {
                        n.collapsed = !n.collapsed;
                    }
                }
                true
            }
            "solo" => {
                if let Some(id) = target.filter(|id| *id != output && graph.soloed_at(*id, 0).is_some()) {
                    self.solo_request = Some(Some((id, 0)));
                    self.solo_shown = Some((id, 0));
                }
                false
            }
            "frame" => {
                let layouts = tidy::layouts(graph, exposed, self.show_previews);
                if let Some(bounds) = self.selected.iter().filter_map(|id| layouts.get(id).map(|l| l.rect)).reduce(|a, b| a.union(b)) {
                    let around = bounds.expand(24.0);
                    graph.frames.push(tequila_materials::graph::Frame {
                        title: "Frame".into(),
                        min: GraphVec2 { x: around.min.x, y: around.min.y - tidy::FRAME_TITLE },
                        max: GraphVec2 { x: around.max.x, y: around.max.y },
                    });
                    true
                } else {
                    false
                }
            }
            "dissolve" => {
                for id in &chosen {
                    dissolve(graph, *id);
                }
                self.clear_selection();
                true
            }
            "delete" => {
                for id in &chosen {
                    graph.remove_node(*id);
                }
                self.clear_selection();
                true
            }
            "add" => {
                self.menu = Some(menu::AddMenu::new(canvas, at, None));
                false
            }
            "tidy" => {
                tidy::arrange(graph, exposed);
                self.frame_pending = true;
                true
            }
            "fit" => {
                self.fit(rect, graph, exposed, None);
                false
            }
            _ => false,
        }
    }

    fn menu_ui(&mut self, ctx: &egui::Context, rect: egui::Rect, project: &Project, graph: &mut MaterialGraph, _layouts: &HashMap<NodeId, NodeLayout>) -> bool {
        let entries = menu::entries(project);
        let Some(state) = &mut self.menu else { return false };
        let from = state.from;
        // Only what can be wired where the menu came from.
        let fits = |kind: &NodeKind| match from {
            _ if matches!(kind, NodeKind::Reroute { .. }) => true,
            None => true,
            Some(End::Output(node, output)) => {
                let Some(ty) = graph.node(node).and_then(|n| n.kind.output_type_at(output)) else { return false };
                kind.inputs().iter().enumerate().any(|(i, (_, input, _))| kind.shows_input(i) && ty.converts_to(*input))
            }
            Some(End::Input(to, socket)) => {
                let Some(target) = graph.node(to).and_then(|n| n.kind.inputs().into_iter().find(|i| i.0 == socket)).map(|i| i.1) else {
                    return false;
                };
                kind.outputs().iter().any(|(_, ty)| ty.converts_to(target))
            }
        };
        let outcome = menu::show(ctx, state, &entries, &fits, rect);
        match outcome {
            menu::Outcome::Open => false,
            menu::Outcome::Close => {
                self.menu = None;
                false
            }
            menu::Outcome::Add(kind) => {
                let Some(state) = self.menu.take() else { return false };
                let mut node = Node::new(kind);
                node.position = GraphVec2 { x: snap(state.at.x), y: snap(state.at.y) };
                let id = graph.add_node(node);
                match state.from {
                    Some(End::Output(from, output)) => {
                        adopt(graph, from, output, id);
                        let ty = graph.node(from).and_then(|n| n.kind.output_type_at(output));
                        let kind = graph.node(id).map(|n| n.kind.clone());
                        if let (Some(ty), Some(kind)) = (ty, kind) {
                            let inputs = kind.inputs();
                            // The same type first, then one that converts.
                            let pick = inputs
                                .iter()
                                .enumerate()
                                .filter(|(i, _)| kind.shows_input(*i))
                                .find(|(_, (_, input, _))| *input == ty)
                                .or_else(|| inputs.iter().enumerate().filter(|(i, _)| kind.shows_input(*i)).find(|(_, (_, input, _))| ty.converts_to(*input)));
                            if let Some((_, (socket, _, _))) = pick {
                                graph.connect_output(from, output, id, *socket);
                            }
                        }
                    }
                    Some(End::Input(to, socket)) => {
                        let target = graph.node(to).and_then(|n| n.kind.inputs().into_iter().find(|i| i.0 == socket)).map(|i| i.1);
                        if let (Some(ty), Some(Node { kind: NodeKind::Reroute { ty: own }, .. })) = (target, graph.node_mut(id)) {
                            *own = ty;
                        }
                        let outputs = graph.node(id).map(|n| n.kind.outputs()).unwrap_or_default();
                        if let Some(ty) = target {
                            let pick = outputs.iter().position(|o| o.1 == ty).or_else(|| outputs.iter().position(|o| o.1.converts_to(ty)));
                            if let Some(index) = pick {
                                graph.connect_output(id, index as u8, to, socket);
                            }
                        }
                    }
                    None => {}
                }
                apart(graph, &[id]);
                self.selected = BTreeSet::from([id]);
                self.active = Some(id);
                true
            }
        }
    }

    /// The bar above the canvas.
    fn toolbar(&mut self, ui: &mut egui::Ui, name: &str, graph: &mut MaterialGraph, exposed: &[Param], committed: &mut bool) {
        let rect = self.canvas_rect;
        ui.horizontal(|ui| {
            let style = ui.style_mut();
            style.spacing.item_spacing.x = 4.0;
            style.spacing.button_padding = egui::vec2(10.0, 3.0);
            style.spacing.interact_size.y = 24.0;
            let button = |text: &str| egui::Button::new(egui::RichText::new(text).size(12.5)).fill(crate::theme::p().control).rounding(6.0);
            // Each side panel's switch is at its own end of the bar.
            ui.toggle_value(&mut self.show_browser, egui::RichText::new("Browser").size(12.5))
                .on_hover_text(format!("The materials, maps and scene materials down the left: drag one into the graph, or double-click it ({})", crate::theme::keys("T")));
            ui.add_space(6.0);
            if ui.add(button("+  Add")).on_hover_text("Add a node (Shift A)").clicked() {
                let at = rect.min + egui::vec2(12.0, 8.0);
                self.menu = Some(menu::AddMenu::new(self.to_canvas(rect, rect.center()), at, None));
            }
            if ui.add(button("Tidy")).on_hover_text("Lay the nodes out in columns flowing into the surface").clicked() {
                tidy::arrange(graph, exposed);
                self.frame_pending = true;
                *committed = true;
            }
            if ui.add(button("Fit")).on_hover_text("Fit the whole graph in view (Home)").clicked() {
                self.fit(rect, graph, exposed, None);
            }
            ui.add_space(6.0);
            ui.toggle_value(&mut self.show_previews, egui::RichText::new("Previews").size(12.5)).on_hover_text("Show each texture's picture in its node");
            ui.toggle_value(&mut self.show_map, egui::RichText::new("Overview").size(12.5))
                .on_hover_text("A small map of the whole graph in the corner, when it does not fit the view: click or drag in it to go there");
            if self.solo_shown.is_some() && ui.add(button("End solo")).on_hover_text("Show the material again").clicked() {
                self.solo_request = Some(None);
                self.solo_shown = None;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut self.show_settings, egui::RichText::new("Settings").size(12.5))
                    .on_hover_text(format!("The material's picture and the selected node's settings down the right ({})", crate::theme::keys("N")));
                ui.add_space(6.0);
                // Whose graph this is: the browser changes it without the
                // window's title being looked at.
                ui.add(egui::Label::new(egui::RichText::new(name).size(12.5).strong()).truncate());
            });
        });
    }
}

/// The canvas's mapping to the screen, copied out so closures can use it
/// while the editor changes.
#[derive(Clone, Copy)]
struct Xf {
    origin: egui::Pos2,
    pan: egui::Vec2,
    zoom: f32,
}

impl Xf {
    fn point(self, p: egui::Pos2) -> egui::Pos2 {
        self.origin + (p.to_vec2() + self.pan) * self.zoom
    }

    fn rect(self, r: egui::Rect) -> egui::Rect {
        egui::Rect::from_min_max(self.point(r.min), self.point(r.max))
    }
}

enum FieldKind {
    Number { int: bool, range: Option<(f32, f32)> },
    Toggle,
    Colour,
    Dropdown(SettingRef),
}

/// Whether the pointer is over the popup drawn as the area `id`.
pub(super) fn over_popup(ctx: &egui::Context, id: &str) -> bool {
    ctx.pointer_interact_pos()
        .or_else(|| ctx.pointer_hover_pos())
        .and_then(|p| ctx.layer_id_at(p))
        .is_some_and(|layer| layer.id == egui::Id::new(id))
}

/// The grid nodes are placed on, in canvas units.
const GRID: f32 = 12.0;

fn snap(v: f32) -> f32 {
    (v / GRID).round() * GRID
}

/// Whether a wire from output `out` of `from` may go into `socket` on `to`,
/// and if not, why.
fn compatible(graph: &MaterialGraph, from: NodeId, out: u8, to: NodeId, socket: Socket) -> (bool, Option<String>) {
    if from == to {
        return (false, Some("A node cannot feed itself".into()));
    }
    if matches!(graph.node(to).map(|n| &n.kind), Some(NodeKind::Reroute { .. })) {
        return (graph.node(from).and_then(|n| n.kind.output_type_at(out)).is_some(), None);
    }
    let output = graph.node(from).and_then(|n| n.kind.output_type_at(out));
    let input = graph.node(to).and_then(|n| n.kind.inputs().into_iter().find(|i| i.0 == socket)).map(|i| i.1);
    match (output, input) {
        (Some(o), Some(i)) if o.converts_to(i) => (true, None),
        (Some(o), Some(i)) => (false, Some(format!("A {} output cannot drive a {} input", o.label(), i.label()))),
        _ => (false, None),
    }
}

/// A reroute takes the type of whatever is wired into it, so the wire keeps
/// its colour and its checks on the far side.
fn adopt(graph: &mut MaterialGraph, from: NodeId, output: u8, to: NodeId) {
    let Some(ty) = graph.node(from).and_then(|n| n.kind.output_type_at(output)) else { return };
    if let Some(Node { kind: NodeKind::Reroute { ty: own }, .. }) = graph.node_mut(to) {
        *own = ty;
    }
}

/// Step `moved` off any node they were put exactly on: two nodes at the very
/// same place are how a graph nobody laid out is known ([`tidy::unplaced`]),
/// and one hidden under another is no use to anybody. A grid step down and
/// right at a time, together, so what was moved keeps its shape.
fn apart(graph: &mut MaterialGraph, moved: &[NodeId]) {
    let place = |p: GraphVec2| (p.x.to_bits(), p.y.to_bits());
    for _ in 0..64 {
        let others: HashSet<(u32, u32)> = graph.nodes().filter(|(id, _)| !moved.contains(id)).map(|(_, n)| place(n.position)).collect();
        let mut own = HashSet::new();
        let clash = moved.iter().filter_map(|id| graph.node(*id)).any(|n| others.contains(&place(n.position)) || !own.insert(place(n.position)));
        if !clash {
            return;
        }
        for (index, id) in moved.iter().enumerate() {
            if let Some(node) = graph.node_mut(*id) {
                // Each a step further when they were on each other too.
                let step = GRID * 2.0 * if own.len() < moved.len() { 1.0 + index as f32 } else { 1.0 };
                node.position = GraphVec2 { x: node.position.x + step, y: node.position.y + step };
            }
        }
    }
}

/// A setup as a graph of its own, laid out by its wires: the nodes it adds,
/// feeding a stand-in surface, and the input of the real surface that what
/// feeds the stand-in is wired to when it is free (`None`: left for the
/// person to wire where they want it).
pub(crate) fn setup_graph(setup: browser::Setup) -> (MaterialGraph, Option<Socket>) {
    use tequila_materials::graph::LAYER_INPUT;
    use tequila_materials::material_type::MaterialType;
    use tequila_materials::nodes::ShaderNode;
    let mut graph = MaterialGraph::from_material(Default::default());
    let surface = graph.output();
    let named = |kind: NodeKind, name: &str| {
        let mut node = Node::new(kind);
        node.label = Some(name.to_string());
        node
    };
    let coat = |name: &str| named(NodeKind::Shader(Box::new(MaterialType::Plastic.defaults())), name);
    match setup {
        browser::Setup::Blend => {
            let stack = graph.add_node(Node::new(NodeKind::Group(tequila_materials::groups::NodeGroup::BlendStack)));
            // Each coat on the Layer of the one under it; the first on the
            // surface's.
            let mut under = surface;
            for index in 0..3u8 {
                let mix = graph.add_node(Node::new(NodeKind::MixShader { add: false }));
                let shader = graph.add_node(coat(&format!("Coat {}", index + 1)));
                graph.connect_output(stack, index, mix, Socket::Input(0));
                graph.connect_output(shader, 0, mix, Socket::Input(1));
                graph.connect_output(mix, 0, under, LAYER_INPUT);
                under = shader;
            }
            (graph, Some(LAYER_INPUT))
        }
        browser::Setup::TwoSided => {
            let geometry = ShaderNode::Geometry;
            let backfacing = geometry.outputs().iter().position(|o| o.name == "Backfacing").unwrap_or(4) as u8;
            let side = graph.add_node(Node::new(NodeKind::Node(geometry)));
            let mix = graph.add_node(Node::new(NodeKind::MixShader { add: false }));
            let back = graph.add_node(coat("Back"));
            graph.connect_output(side, backfacing, mix, Socket::Input(0));
            graph.connect_output(back, 0, mix, Socket::Input(1));
            graph.connect_output(mix, 0, surface, LAYER_INPUT);
            (graph, Some(LAYER_INPUT))
        }
        browser::Setup::Composite => {
            let colour = |name: &str, grey: f32| named(NodeKind::Constant(Value::Color(tequila_core::spectrum::Spectrum::splat(grey))), name);
            let mut under = graph.add_node(colour("Layer 1", 0.8));
            for (index, grey) in [0.5, 0.3, 0.1].into_iter().enumerate() {
                let layer = graph.add_node(colour(&format!("Layer {}", index + 2), grey));
                let mix = graph.add_node(named(NodeKind::Node(ShaderNode::from_key("mix").expect("the Mix node")), &format!("Over {}", index + 1)));
                // Factor, then what is under, then the layer.
                graph.node_mut(mix).expect("just added").literals.insert(Socket::Input(0), Value::Float(0.5));
                graph.connect_output(under, 0, mix, Socket::Input(1));
                graph.connect_output(layer, 0, mix, Socket::Input(2));
                under = mix;
            }
            // Wired to the stand-in for the layout alone.
            graph.connect_output(under, 0, surface, Socket::Param(Param::BaseColor));
            (graph, None)
        }
        browser::Setup::RandomColour => {
            use tequila_core::spectrum::Spectrum;
            use tequila_texture::gradient::{Gradient, Interpolation, Stop};
            let info = graph.add_node(Node::new(NodeKind::Node(ShaderNode::ObjectInfo)));
            // Four bands of the part's number, a colour each.
            let colours = [Spectrum { r: 0.80, g: 0.12, b: 0.10 }, Spectrum { r: 0.90, g: 0.60, b: 0.08 }, Spectrum { r: 0.10, g: 0.45, b: 0.30 }, Spectrum { r: 0.08, g: 0.25, b: 0.65 }];
            let stops = colours.into_iter().enumerate().map(|(index, color)| Stop { at: index as f32 * 0.25, color }).collect();
            let ramp = graph.add_node(named(NodeKind::Gradient { gradient: Box::new(Gradient::new(stops).with_interpolation(Interpolation::Constant)) }, "Colours"));
            graph.connect_output(info, RANDOM, ramp, Socket::Input(0));
            graph.connect_output(ramp, 0, surface, Socket::Param(Param::BaseColor));
            (graph, None)
        }
        browser::Setup::VaryColour => {
            let info = graph.add_node(Node::new(NodeKind::Node(ShaderNode::ObjectInfo)));
            let range = |graph: &mut MaterialGraph, name: &str, low: f32, high: f32| {
                let node = graph.add_node(named(NodeKind::Node(ShaderNode::from_key("map_range").expect("Map Range")), name));
                // Value, From Min, From Max, To Min, To Max.
                let found = graph.node_mut(node).expect("just added");
                found.literals.insert(Socket::Input(3), Value::Float(low));
                found.literals.insert(Socket::Input(4), Value::Float(high));
                node
            };
            // Hue a twentieth either way of where it is, value a fifth.
            let hue = range(&mut graph, "Hue shift", 0.45, 0.55);
            let light = range(&mut graph, "Lightness", 0.8, 1.2);
            let colour = graph.add_node(named(NodeKind::Constant(Value::Color(tequila_core::spectrum::Spectrum { r: 0.75, g: 0.2, b: 0.15 })), "Colour"));
            // Hue, Saturation, Value, Fac, Color.
            let shifted = graph.add_node(Node::new(NodeKind::Node(ShaderNode::HueSaturationValue)));
            graph.connect_output(info, RANDOM, hue, Socket::Input(0));
            graph.connect_output(info, RANDOM, light, Socket::Input(0));
            graph.connect_output(hue, 0, shifted, Socket::Input(0));
            graph.connect_output(light, 0, shifted, Socket::Input(2));
            graph.connect_output(colour, 0, shifted, Socket::Input(4));
            graph.connect_output(shifted, 0, surface, Socket::Param(Param::BaseColor));
            (graph, None)
        }
        browser::Setup::RandomPlacement => {
            use tequila_core::math::vec3;
            use tequila_materials::nodes::Setting;
            let info = graph.add_node(Node::new(NodeKind::Node(ShaderNode::ObjectInfo)));
            let coordinates = graph.add_node(Node::new(NodeKind::Node(ShaderNode::TextureCoordinate)));
            let thrice = graph.add_node(Node::new(NodeKind::Node(ShaderNode::CombineXyz)));
            for axis in 0..3u8 {
                graph.connect_output(info, RANDOM, thrice, Socket::Input(axis));
            }
            // The part's number times something different along each axis:
            // far apart in the pattern, and turned every which way.
            let times = |graph: &mut MaterialGraph, name: &str, by: tequila_core::math::Vec3| {
                let mut multiply = ShaderNode::from_key("vector_math").expect("Vector Math");
                multiply.set("operation", Setting::Choice("multiply"));
                let node = graph.add_node(named(NodeKind::Node(multiply), name));
                graph.node_mut(node).expect("just added").literals.insert(Socket::Input(1), Value::Vector(by));
                graph.connect_output(thrice, 0, node, Socket::Input(0));
                node
            };
            let moved = times(&mut graph, "Move", vec3(13.7, 7.1, 3.3));
            let turned = times(&mut graph, "Turn", vec3(6.283, 12.566, 18.85));
            // Vector, Location, Rotation, Scale.
            let mapping = graph.add_node(Node::new(NodeKind::Node(ShaderNode::from_key("mapping").expect("Mapping"))));
            graph.connect_output(coordinates, OBJECT, mapping, Socket::Input(0));
            graph.connect_output(moved, 0, mapping, Socket::Input(1));
            graph.connect_output(turned, 0, mapping, Socket::Input(2));
            graph.connect_output(mapping, 0, surface, Socket::Param(Param::Tangent));
            (graph, None)
        }
        browser::Setup::StochasticTiling => {
            use tequila_core::spectrum::Spectrum;
            use tequila_materials::graph::TextureId;
            let tiling = graph.add_node(Node::new(NodeKind::Group(tequila_materials::groups::NodeGroup::StochasticTiling)));
            // The picture read three times, where the group says each
            // time. Which picture is the editor's to say
            // (`picture_for_setup`).
            let readings: Vec<NodeId> = ["First reading", "Second reading", "Third reading"]
                .into_iter()
                .enumerate()
                .map(|(output, name)| {
                    let image = graph.add_node(named(NodeKind::ImageTexture { texture: TextureId(0), fallback: Value::Color(Spectrum::splat(0.18)) }, name));
                    graph.connect_output(tiling, output as u8, image, Socket::Input(0));
                    image
                })
                .collect();
            // The second over the first by Blend B, the third over both by
            // Blend C: Factor, then what is under, then what goes over.
            let mut under = readings[0];
            for (index, name) in ["Two readings", "Three readings"].into_iter().enumerate() {
                let mix = graph.add_node(named(NodeKind::Node(ShaderNode::from_key("mix").expect("the Mix node")), name));
                graph.connect_output(tiling, 3 + index as u8, mix, Socket::Input(0));
                graph.connect_output(under, 0, mix, Socket::Input(1));
                graph.connect_output(readings[index + 1], 0, mix, Socket::Input(2));
                under = mix;
            }
            // Wired to the stand-in for the layout alone.
            graph.connect_output(under, 0, surface, Socket::Param(Param::BaseColor));
            (graph, None)
        }
        browser::Setup::Falloff => {
            use tequila_core::spectrum::Spectrum;
            // Layer Weight's Facing: nought seen straight on, one edge on.
            let weight = ShaderNode::LayerWeight;
            let facing = output_named(&weight, "Facing");
            let weight = graph.add_node(Node::new(NodeKind::Node(weight)));
            // Factor, then the colour straight on, then the colour edge on.
            let mix = graph.add_node(
                named(NodeKind::Node(ShaderNode::from_key("mix").expect("the Mix node")), "Front to side")
                    .with_literal(Socket::Input(1), Value::Color(Spectrum { r: 0.05, g: 0.08, b: 0.3 }))
                    .with_literal(Socket::Input(2), Value::Color(Spectrum { r: 0.75, g: 0.85, b: 1.0 })),
            );
            graph.connect_output(weight, facing, mix, Socket::Input(0));
            graph.connect_output(mix, 0, surface, Socket::Param(Param::BaseColor));
            (graph, None)
        }
        browser::Setup::TopBottom => {
            // How far the face looks up: the world normal's Y, from the
            // side (nought) to straight up (one).
            let geometry = ShaderNode::Geometry;
            let normal = output_named(&geometry, "Normal");
            let geometry = graph.add_node(Node::new(NodeKind::Node(geometry)));
            let split = graph.add_node(Node::new(NodeKind::Node(ShaderNode::SeparateXyz)));
            graph.connect_output(geometry, normal, split, Socket::Input(0));
            // Value, From Min, From Max: the top comes on between faces
            // leaning a little up and faces mostly up.
            let range = graph.add_node(
                named(NodeKind::Node(ShaderNode::from_key("map_range").expect("Map Range")), "How far round")
                    .with_literal(Socket::Input(1), Value::Float(0.35))
                    .with_literal(Socket::Input(2), Value::Float(0.65)),
            );
            graph.connect_output(split, 1, range, Socket::Input(0));
            let mix = graph.add_node(Node::new(NodeKind::MixShader { add: false }));
            let top = graph.add_node(coat("Top"));
            graph.connect_output(range, 0, mix, Socket::Input(0));
            graph.connect_output(top, 0, mix, Socket::Input(1));
            graph.connect_output(mix, 0, surface, LAYER_INPUT);
            (graph, Some(LAYER_INPUT))
        }
        browser::Setup::Override => {
            use tequila_materials::nodes::Setting;
            // One for every ray but the camera's.
            let path = ShaderNode::LightPath;
            let camera = output_named(&path, "Is Camera Ray");
            let path = graph.add_node(Node::new(NodeKind::Node(path)));
            let mut subtract = ShaderNode::from_key("math").expect("Math");
            subtract.set("operation", Setting::Choice("subtract"));
            let others = graph.add_node(named(NodeKind::Node(subtract), "Not the camera").with_literal(Socket::Input(0), Value::Float(1.0)));
            graph.connect_output(path, camera, others, Socket::Input(1));
            let mix = graph.add_node(Node::new(NodeKind::MixShader { add: false }));
            let seen = graph.add_node(coat("In reflections"));
            graph.connect_output(others, 0, mix, Socket::Input(0));
            graph.connect_output(seen, 0, mix, Socket::Input(1));
            graph.connect_output(mix, 0, surface, LAYER_INPUT);
            (graph, Some(LAYER_INPUT))
        }
        browser::Setup::ColourCorrection => {
            let colour = graph.add_node(named(NodeKind::Constant(Value::Color(tequila_core::spectrum::Spectrum { r: 0.75, g: 0.2, b: 0.15 })), "Colour in"));
            // Hue, Saturation, Value, Fac, Color.
            let hsv = graph.add_node(Node::new(NodeKind::Node(ShaderNode::HueSaturationValue)));
            graph.connect_output(colour, 0, hsv, Socket::Input(4));
            let contrast = graph.add_node(Node::new(NodeKind::Node(ShaderNode::BrightContrast)));
            graph.connect_output(hsv, 0, contrast, Socket::Input(0));
            let gamma = graph.add_node(Node::new(NodeKind::Node(ShaderNode::Gamma)));
            graph.connect_output(contrast, 0, gamma, Socket::Input(0));
            graph.connect_output(gamma, 0, surface, Socket::Param(Param::BaseColor));
            (graph, None)
        }
        browser::Setup::Distort => {
            use tequila_core::math::Vec3;
            use tequila_materials::nodes::Setting;
            let coordinates = ShaderNode::TextureCoordinate;
            let uv = output_named(&coordinates, "UV");
            let coordinates = graph.add_node(Node::new(NodeKind::Node(coordinates)));
            // A noise of the same coordinates, its three colours a push
            // along each axis: about its middle, so as far one way as the
            // other.
            let noise = ShaderNode::from_key("noise_texture").expect("Noise Texture");
            let colour = output_named(&noise, "Color");
            let noise = graph.add_node(Node::new(NodeKind::Node(noise)).with_literal(Socket::Input(2), Value::Float(4.0)));
            graph.connect_output(coordinates, uv, noise, Socket::Input(0));
            let vector = |graph: &mut MaterialGraph, op: &'static str, name: &str| {
                let mut math = ShaderNode::from_key("vector_math").expect("Vector Math");
                math.set("operation", Setting::Choice(op));
                graph.add_node(named(NodeKind::Node(math), name))
            };
            let centred = vector(&mut graph, "subtract", "About the middle");
            graph.node_mut(centred).expect("just added").literals.insert(Socket::Input(1), Value::Vector(Vec3::splat(0.5)));
            graph.connect_output(noise, colour, centred, Socket::Input(0));
            // Scale's own input, the fourth: how far.
            let pushed = vector(&mut graph, "scale", "Strength");
            graph.node_mut(pushed).expect("just added").literals.insert(Socket::Input(3), Value::Float(0.1));
            graph.connect_output(centred, 0, pushed, Socket::Input(0));
            let distorted = vector(&mut graph, "add", "Distorted");
            graph.connect_output(coordinates, uv, distorted, Socket::Input(0));
            graph.connect_output(pushed, 0, distorted, Socket::Input(1));
            graph.connect_output(distorted, 0, surface, Socket::Param(Param::Tangent));
            (graph, None)
        }
        browser::Setup::Curves => {
            curves_setup(&mut graph, surface);
            (graph, None)
        }
        browser::Setup::WornEdges => {
            use tequila_materials::nodes::Setting;
            let edges = graph.add_node(Node::new(NodeKind::Curvature {
                mode: tequila_materials::graph::CurvatureMode::Convex,
                radius: tequila_materials::graph::DEFAULT_CURVATURE_RADIUS,
                contrast: 1.0,
            }));
            let noise = graph.add_node(named(NodeKind::Node(ShaderNode::from_key("noise_texture").expect("Noise Texture")), "Breaks").with_literal(Socket::Input(2), Value::Float(60.0)));
            let mut multiply = ShaderNode::from_key("math").expect("Math");
            multiply.set("operation", Setting::Choice("multiply"));
            let broken = graph.add_node(named(NodeKind::Node(multiply), "Edges, broken"));
            graph.connect_output(edges, 0, broken, Socket::Input(0));
            graph.connect_output(noise, 0, broken, Socket::Input(1));
            // Value, From Min, From Max: where the wear starts and where
            // it is through.
            let wear = graph.add_node(
                named(NodeKind::Node(ShaderNode::from_key("map_range").expect("Map Range")), "Wear")
                    .with_literal(Socket::Input(1), Value::Float(0.15))
                    .with_literal(Socket::Input(2), Value::Float(0.3)),
            );
            graph.connect_output(broken, 0, wear, Socket::Input(0));
            graph.connect_output(wear, 0, surface, Socket::Param(Param::SpecularRoughness));
            (graph, None)
        }
        browser::Setup::Dirt => {
            use tequila_core::math::vec3;
            use tequila_materials::nodes::Setting;
            let open = graph.add_node(Node::new(NodeKind::Occlusion { radius: tequila_materials::graph::DEFAULT_OCCLUSION_RADIUS, contrast: 1.0 }));
            let math = |graph: &mut MaterialGraph, op: &'static str, name: &str| {
                let mut math = ShaderNode::from_key("math").expect("Math");
                math.set("operation", Setting::Choice(op));
                graph.add_node(named(NodeKind::Node(math), name))
            };
            // Occlusion is one in the open: the creases are what is left.
            let creases = math(&mut graph, "subtract", "Creases");
            graph.node_mut(creases).expect("just added").literals.insert(Socket::Input(0), Value::Float(1.0));
            graph.connect_output(open, 0, creases, Socket::Input(1));
            // A noise squeezed across and drawn out up and down: streaks.
            // Vector, Location, Rotation, Scale.
            let stretch = graph.add_node(named(NodeKind::Node(ShaderNode::from_key("mapping").expect("Mapping")), "Drawn out").with_literal(Socket::Input(3), Value::Vector(vec3(1.0, 0.08, 1.0))));
            let noise = graph.add_node(named(NodeKind::Node(ShaderNode::from_key("noise_texture").expect("Noise Texture")), "Streaks").with_literal(Socket::Input(2), Value::Float(80.0)));
            graph.connect_output(stretch, 0, noise, Socket::Input(0));
            let streaked = math(&mut graph, "multiply", "Creases, streaked");
            graph.connect_output(creases, 0, streaked, Socket::Input(0));
            graph.connect_output(noise, 0, streaked, Socket::Input(1));
            let dirt = graph.add_node(
                named(NodeKind::Node(ShaderNode::from_key("map_range").expect("Map Range")), "Dirt")
                    .with_literal(Socket::Input(1), Value::Float(0.05))
                    .with_literal(Socket::Input(2), Value::Float(0.35)),
            );
            graph.connect_output(streaked, 0, dirt, Socket::Input(0));
            graph.connect_output(dirt, 0, surface, Socket::Param(Param::SpecularRoughness));
            (graph, None)
        }
    }
}

/// The Curves setup: a colour taken apart, each channel through a ramp
/// from black to white, and put together again.
fn curves_setup(graph: &mut MaterialGraph, surface: NodeId) {
    use tequila_core::spectrum::Spectrum;
    use tequila_materials::nodes::ShaderNode;
    use tequila_texture::gradient::{Gradient, Stop};
    let colour = graph.add_node({
        let mut node = Node::new(NodeKind::Constant(Value::Color(Spectrum { r: 0.75, g: 0.2, b: 0.15 })));
        node.label = Some("Colour in".into());
        node
    });
    let apart = graph.add_node(Node::new(NodeKind::Node(ShaderNode::from_key("separate_color").expect("Separate Color"))));
    let together = graph.add_node(Node::new(NodeKind::Node(ShaderNode::from_key("combine_color").expect("Combine Color"))));
    graph.connect_output(colour, 0, apart, Socket::Input(0));
    for (channel, name) in ["Red curve", "Green curve", "Blue curve"].into_iter().enumerate() {
        // Straight, to start with: what goes in comes out.
        let stops = vec![Stop { at: 0.0, color: Spectrum::splat(0.0) }, Stop { at: 1.0, color: Spectrum::splat(1.0) }];
        let mut ramp = Node::new(NodeKind::Gradient { gradient: Box::new(Gradient::new(stops)) });
        ramp.label = Some(name.into());
        let ramp = graph.add_node(ramp);
        graph.connect_output(apart, channel as u8, ramp, Socket::Input(0));
        graph.connect_output(ramp, 0, together, Socket::Input(channel as u8));
    }
    graph.connect_output(together, 0, surface, Socket::Param(Param::BaseColor));
}

/// Which of a node's outputs goes by `name`.
fn output_named(node: &tequila_materials::nodes::ShaderNode, name: &str) -> u8 {
    node.outputs().iter().position(|o| o.name == name).unwrap_or_else(|| panic!("{} has no output {name}", node.label())) as u8
}

/// The picture a setup that reads one starts with: the selected Image's,
/// or the project's first, or one that is missing until the person chooses.
fn picture_for_setup(graph: &MaterialGraph, active: Option<NodeId>, project: &Project) -> tequila_materials::graph::TextureId {
    let chosen = active.and_then(|id| graph.node(id)).and_then(|node| match &node.kind {
        NodeKind::ImageTexture { texture, .. } | NodeKind::ImageAlpha { texture, .. } => Some(*texture),
        _ => None,
    });
    chosen
        .or_else(|| project.document().textures.iter().next().map(|(id, _)| id))
        .unwrap_or(tequila_materials::graph::TextureId(0))
}

/// Object Info's Random output, and Texture Coordinate's Object.
const RANDOM: u8 = 1;
const OBJECT: u8 = 3;

/// Add `other`'s nodes to `graph`, all but its surface, with the wires among
/// them; what fed `other`'s surface feeds `graph`'s at `into`, when that is
/// given and free. The node that fed the surface goes at `at` and the rest
/// keep their places about it. Returns the nodes added, that one last.
fn arrange_setup(graph: &mut MaterialGraph, other: &MaterialGraph, into: Option<Socket>, at: GraphVec2) -> Vec<NodeId> {
    let stand_in = other.output();
    let last = other.links().iter().find(|l| l.to == stand_in).copied();
    let origin = last.and_then(|l| other.node(l.from)).map(|n| n.position).unwrap_or(GraphVec2 { x: 0.0, y: 0.0 });
    let mut fresh: HashMap<NodeId, NodeId> = HashMap::new();
    let mut added = Vec::new();
    for (id, node) in other.nodes().filter(|(id, _)| *id != stand_in) {
        let mut node = node.clone();
        node.position = GraphVec2 { x: snap(node.position.x - origin.x + at.x), y: snap(node.position.y - origin.y + at.y) };
        let new = graph.add_node(node);
        fresh.insert(id, new);
        added.push(new);
    }
    for link in other.links() {
        if let (Some(from), Some(to)) = (fresh.get(&link.from), fresh.get(&link.to)) {
            graph.connect_output(*from, link.from_socket, *to, link.to_socket);
        }
    }
    let surface = graph.output();
    if let (Some(link), Some(socket)) = (last, into) {
        let free = !graph.links().iter().any(|l| l.to == surface && l.to_socket == socket);
        if let (true, Some(from)) = (free, fresh.get(&link.from)) {
            graph.connect_output(*from, link.from_socket, surface, socket);
        }
    }
    // The one that fed the surface last: the one to look at.
    if let Some(top) = last.and_then(|l| fresh.get(&l.from)).copied() {
        added.retain(|id| *id != top);
        added.push(top);
    }
    added
}

/// Another material brought into `graph` whole, to be mixed with it: its
/// nodes and the wires among them, and where its Surface was a Shader of
/// the same surface, named for the material, with the same wires into it -
/// a Shader takes everything a Surface does. Its labels come with it. The
/// Shader goes at `at` and the rest keep their places about it. Returns
/// the Shader and every node added.
fn bring_in(graph: &mut MaterialGraph, other: &MaterialGraph, name: &str, at: GraphVec2) -> (NodeId, Vec<NodeId>) {
    let output = other.output();
    let origin = other.node(output).map(|n| n.position).unwrap_or(GraphVec2 { x: 0.0, y: 0.0 });
    let mut fresh: HashMap<NodeId, NodeId> = HashMap::new();
    let mut added = Vec::new();
    for (id, node) in other.nodes() {
        let mut node = node.clone();
        if id == output {
            if let NodeKind::Surface(material) = &node.kind {
                node.kind = NodeKind::Shader(material.clone());
            }
            node.label = Some(name.to_string());
        }
        node.position = GraphVec2 { x: snap(node.position.x - origin.x + at.x), y: snap(node.position.y - origin.y + at.y) };
        let new = graph.add_node(node);
        fresh.insert(id, new);
        added.push(new);
    }
    for link in other.links() {
        if let (Some(from), Some(to)) = (fresh.get(&link.from), fresh.get(&link.to)) {
            graph.connect_output(*from, link.from_socket, *to, link.to_socket);
        }
    }
    // Its labels are part of it, and come too: printed on this material
    // now, over whatever the two are mixed into, as a label is.
    graph.decals.extend(other.decals.iter().cloned());
    (fresh[&output], added)
}

/// Put `id` into `link`'s wire: the source into its first input that takes
/// the value, its first output that fits into the target.
fn insert(graph: &mut MaterialGraph, link: Link, id: NodeId) {
    let Some(kind) = graph.node(id).map(|n| n.kind.clone()) else { return };
    let carried = graph.node(link.from).and_then(|n| n.kind.output_type_at(link.from_socket));
    let wanted = graph.node(link.to).and_then(|n| n.kind.inputs().into_iter().find(|i| i.0 == link.to_socket)).map(|i| i.1);
    let (Some(carried), Some(wanted)) = (carried, wanted) else { return };
    let inputs = kind.inputs();
    let input = inputs
        .iter()
        .enumerate()
        .filter(|(i, _)| kind.shows_input(*i))
        .find(|(_, i)| i.1 == carried)
        .or_else(|| inputs.iter().enumerate().filter(|(i, _)| kind.shows_input(*i)).find(|(_, i)| carried.converts_to(i.1)))
        .map(|(_, i)| i.0);
    let outputs = kind.outputs();
    let output = outputs.iter().position(|o| o.1 == wanted).or_else(|| outputs.iter().position(|o| o.1.converts_to(wanted)));
    if let (Some(input), Some(output)) = (input, output) {
        adopt(graph, link.from, link.from_socket, id);
        graph.connect_output(link.from, link.from_socket, id, input);
        graph.connect_output(id, output as u8, link.to, link.to_socket);
    }
}

/// Delete a node and join its wires through it, as a mute would pass them.
fn dissolve(graph: &mut MaterialGraph, id: NodeId) {
    let Some(node) = graph.node(id).cloned() else { return };
    let into: Vec<Link> = graph.links().iter().filter(|l| l.to == id).copied().collect();
    let out: Vec<Link> = graph.links().iter().filter(|l| l.from == id).copied().collect();
    graph.remove_node(id);
    for link in out {
        let Some(wanted) = node.kind.output_type_at(link.from_socket) else { continue };
        let source_type = |l: &Link| graph.node(l.from).and_then(|n| n.kind.output_type_at(l.from_socket));
        let pass = into.iter().find(|l| source_type(l) == Some(wanted)).or_else(|| into.iter().find(|l| source_type(l).is_some_and(|t| t.converts_to(wanted))));
        if let Some(pass) = pass.copied() {
            graph.connect_output(pass.from, pass.from_socket, link.to, link.to_socket);
        }
    }
}

fn bezier_points(a: egui::Pos2, b: egui::Pos2, zoom: f32) -> Vec<egui::Pos2> {
    let bend = ((b.x - a.x).abs() * 0.5).max(50.0 * zoom);
    let points = [a, a + egui::vec2(bend, 0.0), b - egui::vec2(bend, 0.0), b];
    egui::epaint::CubicBezierShape::from_points_stroke(points, false, egui::Color32::TRANSPARENT, egui::Stroke::NONE).flatten(Some(1.0))
}

fn segments_cross(p1: egui::Pos2, p2: egui::Pos2, q1: egui::Pos2, q2: egui::Pos2) -> bool {
    let d = |a: egui::Pos2, b: egui::Pos2, c: egui::Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let (d1, d2, d3, d4) = (d(q1, q2, p1), d(q1, q2, p2), d(p1, p2, q1), d(p1, p2, q2));
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
}

fn wire(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2, colour: egui::Color32, zoom: f32, strong: bool) {
    let bend = ((b.x - a.x).abs() * 0.5).max(50.0 * zoom);
    let points = [a, a + egui::vec2(bend, 0.0), b - egui::vec2(bend, 0.0), b];
    let width = (if strong { 2.6 } else { 2.0 }) * zoom.clamp(0.5, 1.6);
    // A dark edge under the wire, so it reads over anything it crosses.
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(points, false, egui::Color32::TRANSPARENT, egui::Stroke::new(width + 2.0, ink::shade(90))));
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(points, false, egui::Color32::TRANSPARENT, egui::Stroke::new(width, colour)));
}

fn socket(painter: &egui::Painter, at: egui::Pos2, ty: SocketType, zoom: f32, filled: bool) {
    let r = (SOCKET_RADIUS * zoom).max(2.5);
    painter.circle_filled(at, r + 1.2, egui::Color32::from_gray(22));
    if filled {
        painter.circle_filled(at, r, view::type_colour(ty));
    } else {
        painter.circle_filled(at, r, view::type_colour(ty).gamma_multiply(0.85));
        painter.circle_filled(at, r * 0.45, ink::body());
    }
}

/// A small triangle: pointing down when `open`, right when not.
fn triangle(painter: &egui::Painter, centre: egui::Pos2, size: f32, open: bool, colour: egui::Color32) {
    let points = if open {
        vec![centre + egui::vec2(-size, -size * 0.55), centre + egui::vec2(size, -size * 0.55), centre + egui::vec2(0.0, size * 0.7)]
    } else {
        vec![centre + egui::vec2(-size * 0.55, -size), centre + egui::vec2(-size * 0.55, size), centre + egui::vec2(size * 0.7, 0.0)]
    };
    painter.add(egui::Shape::convex_polygon(points, colour, egui::Stroke::NONE));
}

/// A number field, as Blender draws one: its name on the left, its value on
/// the right, and for a value with a range the share of it filled in.
#[allow(clippy::too_many_arguments)]
fn number_field(painter: &egui::Painter, area: egui::Rect, z: f32, label: &str, value: f32, int: bool, range: Option<(f32, f32)>, hovered: bool, typing: bool) {
    if typing {
        return;
    }
    let round = 3.0 * z;
    painter.rect_filled(area, round, if hovered { ink::field_hover() } else { ink::field() });
    if let Some((lo, hi)) = range.filter(|(lo, hi)| hi > lo) {
        let share = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
        if share > 0.0 {
            let fill = egui::Rect::from_min_size(area.min, egui::vec2(area.width() * share, area.height()));
            painter.rect_filled(fill, round, ink::fill());
        }
    }
    let font = egui::FontId::proportional((11.5 * z).clamp(5.0, 24.0));
    let text = if int { format!("{}", value.round() as i64) } else { format_value(value) };
    painter.text(area.left_center() + egui::vec2(6.0 * z, 0.0), egui::Align2::LEFT_CENTER, label, font.clone(), ink::label());
    painter.text(area.right_center() - egui::vec2(6.0 * z, 0.0), egui::Align2::RIGHT_CENTER, text, font, ink::text());
}

fn format_value(v: f32) -> String {
    if v.abs() >= 1000.0 {
        format!("{v:.0}")
    } else if v.abs() >= 100.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.3}")
    }
}

fn toggle(painter: &egui::Painter, area: egui::Rect, z: f32, label: &str, on: bool, hovered: bool) {
    let side = (area.height() - 4.0 * z).max(6.0);
    let box_rect = egui::Rect::from_min_size(egui::pos2(area.left(), area.center().y - side * 0.5), egui::vec2(side, side));
    painter.rect_filled(box_rect, 3.0 * z, if on { crate::theme::p().accent.gamma_multiply(0.75) } else if hovered { ink::field_hover() } else { ink::field() });
    if on {
        let c = box_rect.center();
        let s = side * 0.28;
        painter.line_segment([c + egui::vec2(-s, 0.0), c + egui::vec2(-s * 0.2, s * 0.8)], egui::Stroke::new(1.6 * z.max(0.6), egui::Color32::WHITE));
        painter.line_segment([c + egui::vec2(-s * 0.2, s * 0.8), c + egui::vec2(s * 1.1, -s * 0.8)], egui::Stroke::new(1.6 * z.max(0.6), egui::Color32::WHITE));
    }
    painter.text(box_rect.right_center() + egui::vec2(6.0 * z, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional((11.5 * z).clamp(5.0, 24.0)), ink::label());
}

fn colour_field(painter: &egui::Painter, area: egui::Rect, z: f32, label: &str, colour: tequila_core::spectrum::Spectrum, hovered: bool) {
    let font = egui::FontId::proportional((11.5 * z).clamp(5.0, 24.0));
    painter.text(area.left_center(), egui::Align2::LEFT_CENTER, label, font, ink::label());
    let swatch = egui::Rect::from_min_max(egui::pos2(area.left() + area.width() * 0.48, area.top()), area.max);
    let c = tequila_color::DisplayTransform::Srgb.apply_u8(colour);
    painter.rect_filled(swatch, 3.0 * z, egui::Color32::from_rgb(c[0], c[1], c[2]));
    painter.rect_stroke(swatch, 3.0 * z, egui::Stroke::new(1.0_f32, if hovered { egui::Color32::from_gray(200) } else { egui::Color32::from_gray(20) }));
}

/// Turn a refusal from the bus into words for the editor's status line.
fn explain(error: &str) -> String {
    if error.contains("Cycle") {
        "That wire would make a loop: a node cannot depend on its own output.".into()
    } else if error.contains("TypeMismatch") {
        "Those sockets carry different kinds of value.".into()
    } else {
        format!("Not applied: {error}")
    }
}

/// Run the real compiler and shader for one node's first output over a
/// square of surface the model's size, as sRGB pixels.
#[cfg(test)]
fn evaluate_preview(project: &Project, graph: &MaterialGraph, id: NodeId) -> Option<egui::ColorImage> {
    let scene = project.scene();
    let extent = {
        let b = scene.bounds();
        let size = b.max - b.min;
        size.x.max(size.y).max(size.z).clamp(1e-3, 1e3)
    };
    previews::evaluate(&scene.textures, extent, graph, id, previews::preview_output(&graph.node(id)?.kind))
}

/// The line under the canvas: how many nodes, how many of them selected,
/// how close the view is, and the one key to start with, in this
/// computer's names for the keys.
fn status_line(nodes: usize, selected: usize, zoom: f32) -> String {
    let chosen = match selected {
        0 => "none selected".to_string(),
        n => format!("{n} selected"),
    };
    let nodes = if nodes == 1 { "1 node".to_string() } else { format!("{nodes} nodes") };
    format!("{nodes} · {chosen} · {:.0}%   —   {} adds a node; every key is in the sidebar with nothing selected", zoom * 100.0, crate::theme::keys("Shift + A"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::spectrum::Spectrum;

    /// The line under the canvas counts what is there and names one key,
    /// by this computer's name for it: it was a row of six shortcuts with
    /// Ctrl in it on a Mac.
    #[test]
    fn the_status_line_counts_and_names_this_computers_keys() {
        let none = status_line(6, 0, 0.56);
        assert!(none.starts_with("6 nodes · none selected · 56%"), "{none}");
        let one = status_line(1, 1, 1.0);
        assert!(one.starts_with("1 node · 1 selected · 100%"), "{one}");
        assert!(one.contains(&crate::theme::keys("Shift + A")), "{one}");
        assert!(!one.contains("Ctrl"), "{one}");
    }
    use tequila_materials::nodes::ShaderNode;

    fn project() -> Project {
        Project::with_document(
            tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2),
            std::sync::Arc::new(tequila_scene::demo::studio_environment()),
        )
    }

    /// Wiring rules as the editor applies them: a type that converts is
    /// allowed, a flag into a colour is refused with a reason, and a node
    /// never feeds itself.
    #[test]
    fn wires_follow_the_socket_types() {
        let mut graph = MaterialGraph::from_material(Default::default());
        let colour = graph.add_node(Node::new(NodeKind::Constant(Value::Color(Spectrum::splat(0.5)))));
        let flag = graph.add_node(Node::new(NodeKind::Constant(Value::Bool(true))));
        let invert = graph.add_node(Node::new(NodeKind::Invert));
        let surface = graph.output();
        assert!(compatible(&graph, colour, 0, surface, Socket::Param(Param::BaseColor)).0);
        let (ok, why) = compatible(&graph, flag, 0, surface, Socket::Param(Param::BaseColor));
        assert!(!ok && why.is_some(), "a flag is not a colour");
        assert!(!compatible(&graph, invert, 0, invert, Socket::Input(0)).0);
    }

    /// Every pattern's preview is a picture, varying down the square as well
    /// as across it. Flat patterns lie in x–z; a preview square in x–y crossed
    /// them along one line, and scratches previewed blank, weave and mesh as
    /// stripes.
    #[test]
    fn every_pattern_previews_in_both_directions() {
        let project = project();
        let mut kinds: Vec<NodeKind> = tequila_texture::procedural::Procedural::catalog()
            .into_iter()
            .map(|p| NodeKind::Procedural { procedural: Box::new(p), seed: 1 })
            .collect();
        kinds.extend(ShaderNode::catalog().into_iter().filter(|n| n.category() == tequila_materials::nodes::NodeCategory::Texture).map(NodeKind::Node));
        for kind in kinds {
            let mut graph = MaterialGraph::from_material(Default::default());
            let label = kind.label().to_string();
            let node = graph.add_node(Node::new(kind));
            let image = evaluate_preview(&project, &graph, node).expect("a pattern previews");
            let [w, h] = view::PREVIEW_PIXELS;
            let at = |x: usize, y: usize| image.pixels[y * w + x].r() as i32 + image.pixels[y * w + x].g() as i32;
            let across: i32 = (0..h).flat_map(|y| (1..w).map(move |x| (y, x))).map(|(y, x)| (at(x, y) - at(x - 1, y)).abs()).sum();
            let down: i32 = (0..w).flat_map(|x| (1..h).map(move |y| (x, y))).map(|(x, y)| (at(x, y) - at(x, y - 1)).abs()).sum();
            assert!(across > 0 && down > 0, "{label}: changes {across} across and {down} down");
        }
    }

    /// A preview is the renderer's answer: a constant colour node previews
    /// as that colour, through the display transform, in a picture the shape
    /// of the box it is drawn in.
    #[test]
    fn a_preview_is_the_nodes_evaluated_output() {
        let project = project();
        let red = Spectrum::from_linear_srgb(tequila_core::math::vec3(1.0, 0.0, 0.0));
        let mut graph = MaterialGraph::from_material(Default::default());
        let node = graph.add_node(Node::new(NodeKind::Constant(Value::Color(red))));
        let image = evaluate_preview(&project, &graph, node).expect("a colour previews");
        assert_eq!(image.size, view::PREVIEW_PIXELS);
        let p = image.pixels[image.pixels.len() / 2];
        assert!(p.r() > 240 && p.g() < 15 && p.b() < 15, "{p:?}");
        // The box it is drawn in: a texture node's width less its padding,
        // by the preview row's height less its margin.
        let noise = Node::new(NodeKind::Node(ShaderNode::noise()));
        let layout = view::layout(&noise, NodeId(99), &HashSet::new(), &[], true, &|_| String::new());
        let row = layout.rows.iter().find(|r| r.kind == RowKind::Preview).unwrap();
        let area = layout.row_rect(row);
        let [w, h] = view::PREVIEW_PIXELS;
        assert!(((w as f32 / h as f32) - area.width() / (area.height() - 2.0)).abs() < 0.08, "{area:?}");
    }

    /// Dropped on a wire, a node goes in between; deleted keeping the wire,
    /// the wire joins up again.
    #[test]
    fn insert_and_dissolve_rewire() {
        let mut graph = MaterialGraph::from_material(Default::default());
        let colour = graph.add_node(Node::new(NodeKind::Constant(Value::Color(Spectrum::splat(0.4)))));
        let surface = graph.output();
        graph.connect(colour, surface, Socket::Param(Param::BaseColor));
        let link = graph.links()[0];
        let invert = graph.add_node(Node::new(NodeKind::Node(ShaderNode::InvertColor)));
        insert(&mut graph, link, invert);
        assert!(graph.links().iter().any(|l| l.from == colour && l.to == invert && l.to_socket == Socket::Input(1)), "into Color, not Fac");
        assert!(graph.links().iter().any(|l| l.from == invert && l.to == surface));
        dissolve(&mut graph, invert);
        assert!(graph.links().iter().any(|l| l.from == colour && l.to == surface && l.to_socket == Socket::Param(Param::BaseColor)));
    }

    /// The editor driven as a person drives it: real pointer and key
    /// events into a headless egui, the graph read back from the project.
    struct Harness {
        ctx: egui::Context,
        editor: GraphEditor,
        project: Project,
        key: MaterialKey,
        pointer: egui::Pos2,
        time: f64,
        /// The material library the browser lists: none, unless a test
        /// puts some in.
        library: Vec<tequila_format::library::LibraryEntry>,
    }

    impl Harness {
        /// The editor with its browser hidden: the canvas at the width
        /// these tests lay their nodes out in. With it, a node added at the
        /// first empty spot reaches over the surface node.
        fn new() -> Harness {
            Harness::opened(false)
        }

        /// The editor as it opens, the browser down its left side.
        fn with_browser() -> Harness {
            Harness::opened(true)
        }

        fn opened(browser: bool) -> Harness {
            let project = project();
            let key = project.document().materials.iter().next().map(|(k, _)| k).expect("a material");
            let mut editor = GraphEditor::default();
            editor.show_browser = browser;
            editor.open_for(key);
            let mut h = Harness { ctx: egui::Context::default(), editor, project, key, pointer: egui::pos2(10.0, 10.0), time: 0.0, library: Vec::new() };
            h.frame(Vec::new());
            h.frame(Vec::new());
            h
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 1.0 / 60.0;
            let modifiers = events
                .iter()
                .find_map(|e| match e {
                    egui::Event::Key { modifiers, .. } | egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or_default();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                time: Some(self.time),
                modifiers,
                events,
                ..Default::default()
            };
            let (editor, project) = (&mut self.editor, &mut self.project);
            let library = Library { entries: &self.library, picture: &|_| None };
            let _ = self.ctx.run(input, |ctx| editor.show(ctx, project, &library));
        }

        fn graph(&self) -> MaterialGraph {
            self.project.document().materials.get(self.key).unwrap().graph.clone()
        }

        /// Where a canvas point is on screen now.
        fn screen(&self, p: egui::Pos2) -> egui::Pos2 {
            Xf { origin: self.editor.canvas_rect.min, pan: self.editor.pan, zoom: self.editor.zoom }.point(p)
        }

        fn layout_of(&self, id: NodeId) -> NodeLayout {
            let graph = self.graph();
            let definition = self.project.document().materials.get(self.key).unwrap();
            view::layout(graph.node(id).unwrap(), id, &view::connected(&graph), &definition.material_type.exposed(), self.editor.show_previews, &|_| String::new())
        }

        fn move_to(&mut self, p: egui::Pos2) {
            self.pointer = p;
            self.frame(vec![egui::Event::PointerMoved(p)]);
        }

        fn button(&mut self, button: egui::PointerButton, pressed: bool, modifiers: egui::Modifiers) {
            let pos = self.pointer;
            self.frame(vec![egui::Event::PointerButton { pos, button, pressed, modifiers }]);
        }

        /// Press here, move there in steps, let go.
        fn drag(&mut self, from: egui::Pos2, to: egui::Pos2, button: egui::PointerButton, modifiers: egui::Modifiers) {
            self.move_to(from);
            self.button(button, true, modifiers);
            for i in 1..=6 {
                self.move_to(from + (to - from) * (i as f32 / 6.0));
            }
            self.button(button, false, modifiers);
            self.frame(Vec::new());
        }

        fn click(&mut self, at: egui::Pos2, modifiers: egui::Modifiers) {
            self.move_to(at);
            self.button(egui::PointerButton::Primary, true, modifiers);
            self.button(egui::PointerButton::Primary, false, modifiers);
            self.frame(Vec::new());
        }

        fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
            self.frame(vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
            self.frame(vec![egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }]);
        }

        fn text(&mut self, text: &str) {
            self.frame(vec![egui::Event::Text(text.into())]);
        }

        /// Somewhere on the canvas with nothing on it.
        fn empty_spot(&self) -> egui::Pos2 {
            let canvas = self.editor.canvas_rect;
            let graph = self.graph();
            let taken: Vec<egui::Rect> = graph.nodes().map(|(id, _)| self.layout_of(id).rect).map(|r| egui::Rect::from_min_max(self.screen(r.min), self.screen(r.max)).expand(30.0)).collect();
            (0..40)
                .flat_map(|i| (0..30).map(move |j| egui::pos2(canvas.left() + 40.0 + i as f32 * 20.0, canvas.top() + 60.0 + j as f32 * 20.0)))
                .find(|p| canvas.shrink(30.0).contains(*p) && !taken.iter().any(|r| r.contains(*p)))
                .expect("room on the canvas")
        }
    }

    /// With the graph out of view, the minimap shows; a click on a node's
    /// block in it brings that node to the middle of the canvas, and selects
    /// nothing on the canvas underneath.
    #[test]
    fn the_minimap_takes_the_view_to_a_node() {
        let mut h = Harness::new();
        h.frame(Vec::new());
        assert!(h.editor.minimap.is_none(), "a graph that fits needs no map");
        // Look far away from everything.
        h.editor.pan += egui::vec2(4000.0, 2500.0);
        h.frame(Vec::new());
        h.frame(Vec::new());
        let map = h.editor.minimap.expect("the map shows when the graph is out of view");
        let surface = h.graph().output();
        let centre = h.layout_of(surface).rect.center();
        let selected = h.editor.selected.clone();
        h.click(map.to_map(centre), egui::Modifiers::NONE);
        let now = h.screen(centre);
        let middle = h.editor.canvas_rect.center();
        assert!((now - middle).length() < 3.0, "the surface is in the middle: {now:?} against {middle:?}");
        assert_eq!(h.editor.selected, selected, "the click was the map's, not the canvas's");
    }

    /// Shift A over the canvas, type a name, Enter: the node is in the
    /// graph where the pointer was, and nothing else changed.
    #[test]
    fn shift_a_and_typing_adds_a_node() {
        let mut h = Harness::new();
        let before = h.graph().len();
        let spot = h.empty_spot();
        h.move_to(spot);
        h.key(egui::Key::A, egui::Modifiers::SHIFT);
        assert!(h.editor.menu.is_some(), "Shift A opens the Add menu");
        h.text("voronoi");
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        let graph = h.graph();
        assert_eq!(graph.len(), before + 1, "one node added");
        assert!(graph.nodes().any(|(_, n)| matches!(&n.kind, NodeKind::Node(s) if s.key() == "voronoi_texture")), "the Voronoi was added");
        assert!(h.editor.menu.is_none());
    }

    /// Dragging from a node's second output to the surface's colour wires
    /// that output, and nothing else.
    #[test]
    fn dragging_from_an_output_wires_it() {
        let mut h = Harness::new();
        let spot = h.empty_spot();
        h.move_to(spot);
        h.key(egui::Key::A, egui::Modifiers::SHIFT);
        h.text("noise texture");
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        let graph = h.graph();
        let noise = graph.nodes().find(|(_, n)| matches!(&n.kind, NodeKind::Node(s) if s.key() == "noise_texture")).map(|(id, _)| id).unwrap();
        let output = graph.output();
        let from = h.screen(h.layout_of(noise).output_at(1).unwrap());
        let to = h.screen(h.layout_of(output).input_at(Socket::Param(Param::BaseColor)).unwrap());
        h.drag(from, to, egui::PointerButton::Primary, egui::Modifiers::NONE);
        let graph = h.graph();
        assert!(
            graph.links().iter().any(|l| l.from == noise && l.from_socket == 1 && l.to == output && l.to_socket == Socket::Param(Param::BaseColor)),
            "the colour output is wired into the surface: {:?}",
            graph.links()
        );
    }

    /// A box drawn round nodes selects them; X deletes them, and never the
    /// surface; M mutes.
    #[test]
    fn box_select_mute_and_delete() {
        let mut h = Harness::new();
        for name in ["checker", "magic"] {
            let spot = h.empty_spot();
            h.move_to(spot);
            h.key(egui::Key::A, egui::Modifiers::SHIFT);
            h.text(name);
            h.key(egui::Key::Enter, egui::Modifiers::NONE);
        }
        let graph = h.graph();
        let added: Vec<NodeId> = graph.nodes().filter(|(_, n)| matches!(&n.kind, NodeKind::Node(_))).map(|(id, _)| id).collect();
        assert_eq!(added.len(), 2);
        // A box round both, from empty space.
        let rects: Vec<egui::Rect> = added.iter().map(|id| h.layout_of(*id).rect).map(|r| egui::Rect::from_min_max(h.screen(r.min), h.screen(r.max))).collect();
        let around = rects[0].union(rects[1]).expand(12.0);
        h.drag(around.left_top(), around.right_bottom(), egui::PointerButton::Primary, egui::Modifiers::NONE);
        for id in &added {
            assert!(h.editor.selected.contains(id), "the box selected {id:?}: {:?}", h.editor.selected);
        }
        h.move_to(around.center());
        h.key(egui::Key::M, egui::Modifiers::NONE);
        assert!(added.iter().all(|id| h.graph().node(*id).unwrap().muted), "M mutes the selection");
        h.key(egui::Key::X, egui::Modifiers::NONE);
        let graph = h.graph();
        assert!(added.iter().all(|id| graph.node(*id).is_none()), "X deletes the selection");
        assert!(graph.node(graph.output()).is_some(), "the surface stays");
    }

    /// Typing in a text field does not reach the canvas: Backspace in the
    /// Add menu's search deletes a letter, not the selected node.
    #[test]
    fn backspace_in_a_field_does_not_delete_a_node() {
        let mut h = Harness::new();
        let spot = h.empty_spot();
        h.move_to(spot);
        h.key(egui::Key::A, egui::Modifiers::SHIFT);
        h.text("brick");
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        let count = h.graph().len();
        assert_eq!(h.editor.selected.len(), 1, "the new node is selected");
        let spot = h.empty_spot();
        h.move_to(spot);
        h.key(egui::Key::A, egui::Modifiers::SHIFT);
        h.text("ab");
        h.key(egui::Key::Backspace, egui::Modifiers::NONE);
        assert_eq!(h.graph().len(), count, "Backspace while searching deleted a node");
        assert_eq!(h.editor.menu.as_ref().map(|m| m.query.as_str()), Some("a"));
    }

    /// A number field scrubs when dragged, and takes typing when clicked.
    #[test]
    fn a_number_field_drags_and_types() {
        let mut h = Harness::new();
        let spot = h.empty_spot();
        h.move_to(spot);
        h.key(egui::Key::A, egui::Modifiers::SHIFT);
        h.text("noise texture");
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        let noise = h.graph().nodes().find(|(_, n)| matches!(&n.kind, NodeKind::Node(_))).map(|(id, _)| id).unwrap();
        let layout = h.layout_of(noise);
        let row = layout.rows.iter().find(|r| r.kind == RowKind::Input(2)).unwrap().clone();
        let field = layout.row_rect(&row);
        let centre = h.screen(field.center());
        let scale = |h: &Harness| view::read(&h.graph(), Target::Input { node: noise, index: 2, component: None }).unwrap().as_float();
        let before = scale(&h);
        h.drag(centre, centre + egui::vec2(60.0, 0.0), egui::PointerButton::Primary, egui::Modifiers::NONE);
        let dragged = scale(&h);
        assert!(dragged > before, "dragging right raised Scale from {before} to {dragged}");
        h.click(centre, egui::Modifiers::NONE);
        assert!(h.editor.typing.is_some(), "a click opens the field for typing");
        h.key(egui::Key::A, egui::Modifiers::COMMAND);
        h.text("7.5");
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        assert!((scale(&h) - 7.5).abs() < 1e-4, "typed 7.5, got {}", scale(&h));
    }

    /// Ctrl and a right-drag across a wire cuts it.
    #[test]
    fn a_ctrl_right_drag_cuts_wires() {
        let mut h = Harness::new();
        let spot = h.empty_spot();
        h.move_to(spot);
        h.key(egui::Key::A, egui::Modifiers::SHIFT);
        h.text("checker");
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        let graph = h.graph();
        let checker = graph.nodes().find(|(_, n)| matches!(&n.kind, NodeKind::Node(_))).map(|(id, _)| id).unwrap();
        let output = graph.output();
        let from = h.screen(h.layout_of(checker).output_at(0).unwrap());
        let to = h.screen(h.layout_of(output).input_at(Socket::Param(Param::BaseColor)).unwrap());
        h.drag(from, to, egui::PointerButton::Primary, egui::Modifiers::NONE);
        assert_eq!(h.graph().links().len(), 1, "wired");
        // Across the middle of the wire, top to bottom.
        let middle = from + (to - from) * 0.5;
        h.drag(middle - egui::vec2(0.0, 60.0), middle + egui::vec2(0.0, 60.0), egui::PointerButton::Secondary, egui::Modifiers::CTRL);
        assert!(h.graph().links().is_empty(), "the cut removed the wire: {:?}", h.graph().links());
    }

    /// A setting's dropdown opens on a click and takes a choice.
    #[test]
    fn a_dropdown_takes_a_choice() {
        let mut h = Harness::new();
        let spot = h.empty_spot();
        h.move_to(spot);
        h.key(egui::Key::A, egui::Modifiers::SHIFT);
        h.text("noise texture");
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        let noise = h.graph().nodes().find(|(_, n)| matches!(&n.kind, NodeKind::Node(_))).map(|(id, _)| id).unwrap();
        let layout = h.layout_of(noise);
        // The second setting: the fractal type.
        let row = layout.rows.iter().find(|r| r.kind == RowKind::Setting(SettingRef::Shader(1))).unwrap().clone();
        let field = h.screen(layout.row_rect(&row).center());
        h.click(field, egui::Modifiers::NONE);
        let dropdown = h.editor.dropdown.as_ref().expect("the dropdown opened");
        // Its options are listed below the field; "Ridged" among them.
        let at = dropdown.at;
        let _ = at;
        assert!(view::choose(&mut h.graph(), noise, SettingRef::Shader(1), "ridged_multifractal"));
        // Choose through the popup: the fourth option, one row each.
        let options = view::choices(h.graph().node(noise).unwrap(), SettingRef::Shader(1)).unwrap().0;
        let index = options.iter().position(|(k, _)| *k == "ridged_multifractal").unwrap();
        // Rows as egui lays them: its interact height apart, plus spacing,
        // below the popup's gap, margin and stroke.
        let style = h.ctx.style();
        let pitch = style.spacing.interact_size.y + style.spacing.item_spacing.y;
        let top = 2.0 + 4.0 + 1.0;
        let target = h.editor.dropdown.as_ref().unwrap().at + egui::vec2(30.0, top + pitch * index as f32 + style.spacing.interact_size.y * 0.5);
        h.click(target, egui::Modifiers::NONE);
        let kind = h.graph().node(noise).unwrap().kind.clone();
        assert!(matches!(kind, NodeKind::Node(ShaderNode::Noise { fractal: tequila_texture::cycles::noise::FractalKind::RidgedMultifractal, .. })), "{kind:?}");
        assert!(h.editor.dropdown.is_none(), "it closes on choosing");
    }

    /// Copy and paste keeps the wires among what was copied, and nothing
    /// else.
    #[test]
    fn paste_keeps_the_wires_inside() {
        let mut editor = GraphEditor::default();
        let mut graph = MaterialGraph::from_material(Default::default());
        let noise = graph.add_node(Node::new(NodeKind::Node(ShaderNode::noise())));
        let ramp = graph.add_node(Node::new(NodeKind::Gradient { gradient: Box::default() }));
        let surface = graph.output();
        graph.connect_output(noise, 0, ramp, Socket::Input(0));
        graph.connect(ramp, surface, Socket::Param(Param::BaseColor));
        editor.copy(&graph, &[noise, ramp]);
        assert!(editor.paste(&mut graph, egui::vec2(300.0, 300.0)));
        assert_eq!(graph.len(), 5);
        let fresh: Vec<NodeId> = editor.selected.iter().copied().collect();
        assert_eq!(fresh.len(), 2);
        assert_eq!(graph.links().iter().filter(|l| fresh.contains(&l.from) && fresh.contains(&l.to)).count(), 1);
        assert!(!graph.links().iter().any(|l| fresh.contains(&l.from) && l.to == surface), "the copy is not wired to the surface");
    }
    /// What the browser asks is done as the Add menu does it: a
    /// double-clicked row is a node in the middle of the view, selected,
    /// and the next one beside it and not on it; a dropped one is under
    /// the pointer; a Materials row is a Shader named for its kind.
    #[test]
    fn the_browser_adds_nodes_where_they_are_put() {
        let mut h = Harness::with_browser();
        let mut graph = h.graph();
        let before = graph.len();
        let canvas = h.editor.canvas_rect;
        let value = || NodeKind::Constant(Value::Float(0.5));

        assert!(h.editor.browse(browser::Act::Add { kind: value(), label: None, at: None }, &mut graph, &h.project, &Library::none(), &[]));
        assert_eq!(graph.len(), before + 1);
        let first = h.editor.active.expect("the new node is the active one");
        assert_eq!(h.editor.selected, BTreeSet::from([first]));
        fn at(h: &Harness, graph: &MaterialGraph, id: NodeId) -> egui::Pos2 {
            let p = graph.node(id).unwrap().position;
            h.screen(egui::pos2(p.x, p.y))
        }
        let middle = at(&h, &graph, first);
        assert!(canvas.shrink(40.0).contains(middle), "{middle:?} is in the view {canvas:?}");
        assert!((middle.x - canvas.center().x).abs() < 160.0 && (middle.y - canvas.center().y).abs() < 160.0, "{middle:?} is near the middle of {canvas:?}");

        assert!(h.editor.browse(browser::Act::Add { kind: value(), label: None, at: None }, &mut graph, &h.project, &Library::none(), &[]));
        let second = h.editor.active.unwrap();
        assert_ne!(at(&h, &graph, second), middle, "the second landed on the first");

        let drop = canvas.left_top() + egui::vec2(300.0, 200.0);
        assert!(h.editor.browse(browser::Act::Add { kind: value(), label: None, at: Some(drop) }, &mut graph, &h.project, &Library::none(), &[]));
        let dropped = at(&h, &graph, h.editor.active.unwrap());
        let width = view::node_width(&value()) * h.editor.zoom;
        assert!(dropped.x <= drop.x && drop.x <= dropped.x + width + 20.0, "{dropped:?} is not under {drop:?}");
        assert!((dropped.y - drop.y).abs() < 40.0, "{dropped:?} is not under {drop:?}");

        let glass = tequila_materials::material_type::MaterialType::Glass;
        let shader = NodeKind::Shader(Box::new(glass.defaults()));
        assert!(h.editor.browse(browser::Act::Add { kind: shader, label: Some(glass.label().into()), at: None }, &mut graph, &h.project, &Library::none(), &[]));
        let node = graph.node(h.editor.active.unwrap()).unwrap();
        assert_eq!(view::title(node), glass.label(), "a Materials row is named for its kind");
        assert!(!h.editor.browse(browser::Act::None, &mut graph, &h.project, &Library::none(), &[]));
    }

    /// With the pointer, as it is used: a row dragged out of the browser
    /// and let go over the canvas is a node there, one let go anywhere else
    /// is nothing, and a double-click adds one too.
    #[test]
    fn a_row_is_dragged_out_of_the_browser_into_the_graph() {
        let mut h = Harness::with_browser();
        h.editor.browser.query = "rgb".into();
        h.frame(Vec::new());
        h.frame(Vec::new());
        let row = |h: &Harness| h.editor.browser.drawn.iter().find(|(label, _)| label == "RGB").map(|(_, rect)| rect.center()).expect("the RGB row is drawn");
        let colours = |h: &Harness| h.graph().nodes().filter(|(_, n)| matches!(n.kind, NodeKind::Constant(Value::Color(_)))).map(|(id, _)| id).collect::<Vec<_>>();
        assert!(colours(&h).is_empty());
        let from = row(&h);
        assert!(!h.editor.canvas_rect.contains(from), "the browser is beside the canvas, not on it");

        let drop = h.empty_spot();
        h.drag(from, drop, egui::PointerButton::Primary, egui::Modifiers::NONE);
        let added = colours(&h);
        assert_eq!(added.len(), 1, "the dragged row became a node");
        assert_eq!(h.editor.active, Some(added[0]));
        let rect = h.layout_of(added[0]).rect;
        let on_screen = egui::Rect::from_min_max(h.screen(rect.min), h.screen(rect.max));
        assert!(on_screen.expand(24.0).contains(drop), "{on_screen:?} is not where it was dropped, {drop:?}");

        // Let go back on the browser: nothing.
        let from = row(&h);
        h.drag(from, from + egui::vec2(10.0, 60.0), egui::PointerButton::Primary, egui::Modifiers::NONE);
        assert_eq!(colours(&h).len(), 1);

        let from = row(&h);
        h.move_to(from);
        for pressed in [true, false, true, false] {
            h.button(egui::PointerButton::Primary, pressed, egui::Modifiers::NONE);
        }
        h.frame(Vec::new());
        assert_eq!(colours(&h).len(), 2, "a double-click adds one");
        // One undo step each: the first Ctrl+Z takes the second node only.
        h.project.undo();
        assert_eq!(colours(&h).len(), 1);
    }

    /// A scene material clicked in the browser is the one the editor is on
    /// from the next frame, and one that has gone is not.
    #[test]
    fn the_browser_opens_another_of_the_scenes_materials() {
        let mut h = Harness::with_browser();
        // The part given a copy of its material: two in the scene.
        let part = h.project.document().tree.iter().find(|n| matches!(n.kind, tequila_scene::tree::NodeKind::Part { .. })).map(|n| n.id).expect("a part");
        h.project.apply(Command::UnlinkMaterial { node: part }).expect("unlinked");
        let added = h.project.document().tree.get(part).unwrap().material.expect("the part's own material");
        assert_ne!(added, h.key);
        assert!(h.project.document().materials.get(h.key).is_some(), "the first material is still in the scene");
        h.frame(Vec::new());

        let mut graph = h.graph();
        assert!(!h.editor.browse(browser::Act::Open(added), &mut graph, &h.project, &Library::none(), &[]), "opening changes no graph");
        assert_eq!(h.editor.material(), Some(h.key), "not until this frame's edits are applied");
        h.frame(Vec::new());
        assert_eq!(h.editor.material(), Some(added));
        assert!(h.editor.open);

        assert!(!h.editor.browse(browser::Act::Open(MaterialKey(987_654)), &mut graph, &h.project, &Library::none(), &[]));
        h.frame(Vec::new());
        assert_eq!(h.editor.material(), Some(added), "a material that is not there is not opened");
    }

    /// T hides the browser and shows it, N the settings, as Blender's side
    /// panels: with the pointer over the canvas, and no modifier.
    #[test]
    fn t_and_n_hide_the_side_panels() {
        let mut h = Harness::with_browser();
        let spot = h.empty_spot();
        h.move_to(spot);
        assert!(h.editor.show_browser && h.editor.show_settings);
        h.key(egui::Key::T, egui::Modifiers::NONE);
        assert!(!h.editor.show_browser);
        assert!(h.editor.show_settings);
        let spot = h.empty_spot();
        h.move_to(spot);
        h.key(egui::Key::N, egui::Modifiers::NONE);
        assert!(!h.editor.show_settings);
        let spot = h.empty_spot();
        h.move_to(spot);
        h.key(egui::Key::T, egui::Modifiers::NONE);
        // The browser is back and the canvas narrower: over it again.
        let spot = h.empty_spot();
        h.move_to(spot);
        h.key(egui::Key::N, egui::Modifiers::NONE);
        assert!(h.editor.show_browser && h.editor.show_settings);
        let spot = h.empty_spot();
        h.move_to(spot);
        h.key(egui::Key::T, egui::Modifiers::SHIFT);
        assert!(h.editor.show_browser, "Shift+T is not T");
    }
    /// A material's graph with a picture into its colour: an image node
    /// wired to the surface, as a library material with a map has.
    fn mapped(texture: u32) -> MaterialGraph {
        let mut graph = MaterialGraph::from_material(tequila_materials::material_type::MaterialType::Metal.defaults());
        let image = graph.add_node(Node::new(NodeKind::ImageTexture { texture: tequila_materials::graph::TextureId(texture), fallback: Value::Color(Spectrum::splat(0.18)) }));
        let surface = graph.output();
        graph.connect_output(image, 0, surface, Socket::Param(Param::BaseColor));
        graph
    }

    /// A material brought in whole: its nodes and their wires, its surface
    /// now a Shader of the same surface under the material's name, and the
    /// graph it came into otherwise as it was.
    #[test]
    fn a_material_is_brought_in_as_a_shader_with_its_nodes() {
        let other = mapped(7);
        let mut graph = MaterialGraph::from_material(Default::default());
        let surface = graph.output();
        let before = graph.len();
        let (shader, fresh) = bring_in(&mut graph, &other, "Brass", GraphVec2 { x: 480.0, y: 240.0 });
        assert_eq!(graph.len(), before + other.len());
        assert_eq!(fresh.len(), other.len());
        assert_eq!(graph.output(), surface, "the graph's own surface is still its output");
        let node = graph.node(shader).unwrap();
        let NodeKind::Shader(material) = &node.kind else { panic!("the surface came in as {:?}", node.kind.label()) };
        assert_eq!(**material, *other.material(), "the same surface");
        assert_eq!(view::title(node), "Brass");
        assert_eq!((node.position.x, node.position.y), (480.0, 240.0), "the Shader is where it was put");
        let image = fresh.iter().copied().find(|id| matches!(graph.node(*id).unwrap().kind, NodeKind::ImageTexture { .. })).expect("the image node came too");
        assert!(
            graph.links().iter().any(|l| l.from == image && l.to == shader && l.to_socket == Socket::Param(Param::BaseColor)),
            "the picture is wired into the Shader's colour: {:?}",
            graph.links()
        );
        assert!(!graph.links().iter().any(|l| l.to == surface), "nothing was wired into the graph's own surface");
        graph.compile().expect("it compiles as it came in");

        // Its labels are part of it: they come too, after the graph's own.
        let mut labelled = mapped(7);
        labelled.decals.push(tequila_materials::decal::Decal::new("Logo", tequila_materials::graph::TextureId(9)));
        let mut graph = MaterialGraph::from_material(Default::default());
        graph.decals.push(tequila_materials::decal::Decal::new("Own", tequila_materials::graph::TextureId(3)));
        bring_in(&mut graph, &labelled, "Brass", GraphVec2 { x: 480.0, y: 240.0 });
        assert_eq!(graph.decals.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Own", "Logo"]);
    }

    /// With the pointer: a scene material's row dragged into the graph is
    /// that material brought in, in a frame with its name, as one undo
    /// step; and the row of the material being edited does not drag.
    #[test]
    fn a_scene_material_is_dragged_into_the_graph() {
        let mut h = Harness::with_browser();
        let part = h.project.document().tree.iter().find(|n| matches!(n.kind, tequila_scene::tree::NodeKind::Part { .. })).map(|n| n.id).expect("a part");
        h.project.apply(Command::UnlinkMaterial { node: part }).expect("unlinked");
        let other = h.project.document().tree.get(part).unwrap().material.expect("the part's own material");
        h.project.apply(Command::RenameMaterial { material: other, name: "Second".into() }).expect("renamed");
        let mut its = h.project.document().materials.get(other).unwrap().graph.clone();
        let extra = its.add_node(Node::new(NodeKind::Constant(Value::Color(Spectrum::splat(0.5)))));
        let surface = its.output();
        its.connect_output(extra, 0, surface, Socket::Param(Param::BaseColor));
        h.project.apply(Command::SetGraph { material: other, graph: Box::new(its.clone()) }).expect("a graph");
        h.project.end_interaction();
        // Searched for, so its row is in view: the scene's materials are
        // under the maps, below the fold.
        h.editor.browser.query = "second".into();
        h.frame(Vec::new());
        h.frame(Vec::new());

        let row = |h: &Harness, name: &str| h.editor.browser.drawn.iter().find(|(label, _)| label == name).map(|(_, rect)| rect.center()).unwrap_or_else(|| panic!("no row {name}: {:?}", h.editor.browser.drawn.iter().map(|(l, _)| l).collect::<Vec<_>>()));
        let before = h.graph().len();
        let drop = h.empty_spot();
        let from = row(&h, "Second");
        h.drag(from, drop, egui::PointerButton::Primary, egui::Modifiers::NONE);
        assert_eq!(h.editor.material(), Some(h.key), "a drag does not open it");
        assert_eq!(h.editor.message, None, "the graph was taken");
        let graph = h.graph();
        assert_eq!(graph.len(), before + its.len(), "its nodes came in");
        let shader = graph.nodes().find(|(_, n)| matches!(n.kind, NodeKind::Shader(_))).map(|(id, _)| id).expect("a Shader");
        assert_eq!(view::title(graph.node(shader).unwrap()), "Second");
        assert!(graph.links().iter().any(|l| l.to == shader && l.to_socket == Socket::Param(Param::BaseColor)));
        assert_eq!(graph.frames.iter().filter(|f| f.title == "Second").count(), 1, "framed, with its name");
        h.project.undo();
        assert_eq!(h.graph().len(), before, "one undo takes all of it back");

        // The material the editor is on: its own row stays put.
        let own = h.project.document().materials.get(h.key).unwrap().name.clone();
        h.editor.browser.query = own.clone();
        h.frame(Vec::new());
        h.frame(Vec::new());
        // The last row by that name: a kind of material may share it, and
        // the scene's are listed after those.
        let from = h.editor.browser.drawn.iter().rev().find(|(label, _)| *label == own).map(|(_, rect)| rect.center()).expect("its own row");
        let drop = h.empty_spot();
        h.drag(from, drop, egui::PointerButton::Primary, egui::Modifiers::NONE);
        assert_eq!(h.graph().len(), before);
    }

    /// A library material double-clicked in the browser comes in the same
    /// way, and the picture it reads is added to the document with it, the
    /// graph reading it by the document's number: all one undo step.
    #[test]
    fn a_library_material_comes_in_with_its_pictures() {
        let mut h = Harness::with_browser();
        let material = tequila_materials::material_type::MaterialType::Metal.defaults();
        h.library = vec![tequila_format::library::LibraryEntry {
            name: "Brass Hairline".into(),
            category: "Metal".into(),
            tags: Vec::new(),
            material_type: tequila_materials::material_type::MaterialType::Metal,
            material,
            graph: mapped(1),
            textures: vec![(tequila_materials::graph::TextureId(1), tequila_scene::textures::TextureSource::new("brass-hairline.png", tequila_texture::texture::MapRole::Color))],
            preview_uv_unit: None,
            source: tequila_format::library::Source::Standard,
            path: String::new(),
        }];
        h.editor.browser.query = "brass".into();
        h.frame(Vec::new());
        h.frame(Vec::new());
        let pictures = |h: &Harness| h.project.document().textures.iter().count();
        let (before, had) = (h.graph().len(), pictures(&h));
        let row = h.editor.browser.drawn.iter().find(|(label, _)| label == "Brass Hairline").map(|(_, rect)| rect.center()).expect("the library's row");
        h.move_to(row);
        for pressed in [true, false, true, false] {
            h.button(egui::PointerButton::Primary, pressed, egui::Modifiers::NONE);
        }
        h.frame(Vec::new());

        let graph = h.graph();
        assert_eq!(graph.len(), before + 2, "the Shader and its image node");
        assert_eq!(pictures(&h), had + 1, "its picture is in the document");
        let (added, source) = h.project.document().textures.iter().last().map(|(id, s)| (id, s.clone())).unwrap();
        assert!(source.path.ends_with("brass-hairline.png"));
        let reads = graph.nodes().find_map(|(_, n)| match n.kind {
            NodeKind::ImageTexture { texture, .. } => Some(texture),
            _ => None,
        });
        assert_eq!(reads, Some(added), "the node reads the document's picture");
        assert!(graph.nodes().any(|(_, n)| matches!(n.kind, NodeKind::Shader(_)) && view::title(n) == "Brass Hairline"));

        h.project.undo();
        assert_eq!((h.graph().len(), pictures(&h)), (before, had), "one undo takes the nodes and the picture back");
    }

    /// A row let go over a wire it fits goes in between, as a node dragged
    /// onto one does; over empty canvas it is wired to nothing.
    #[test]
    fn a_row_dropped_on_a_wire_goes_in_between() {
        let mut h = Harness::with_browser();
        // A colour wired into the surface's colour, well apart.
        let mut graph = h.graph();
        let surface = graph.output();
        let at = graph.node(surface).unwrap().position;
        let mut colour = Node::new(NodeKind::Constant(Value::Color(Spectrum::splat(0.5))));
        colour.position = GraphVec2 { x: at.x - 520.0, y: at.y };
        let colour = graph.add_node(colour);
        graph.connect_output(colour, 0, surface, Socket::Param(Param::BaseColor));
        h.project.apply(Command::SetGraph { material: h.key, graph: Box::new(graph) }).expect("a graph");
        h.project.end_interaction();
        h.editor.frame_pending = true;
        h.editor.browser.query = "invert".into();
        h.frame(Vec::new());
        h.frame(Vec::new());

        let from = h.screen(h.layout_of(colour).output_at(0).unwrap());
        let to = h.screen(h.layout_of(surface).input_at(Socket::Param(Param::BaseColor)).unwrap());
        let middle = from + (to - from) * 0.5;
        assert!(h.editor.canvas_rect.contains(middle), "the wire's middle is in view");
        let (label, row) = h.editor.browser.drawn.iter().find(|(label, _)| label.to_lowercase().starts_with("invert")).map(|(label, rect)| (label.clone(), rect.center())).expect("an Invert row");
        h.drag(row, middle, egui::PointerButton::Primary, egui::Modifiers::NONE);

        let graph = h.graph();
        let added = graph.nodes().map(|(id, _)| id).find(|id| *id != colour && *id != surface).unwrap_or_else(|| panic!("{label} was not added"));
        assert!(graph.links().iter().any(|l| l.from == colour && l.to == added), "the colour goes into it: {:?}", graph.links());
        assert!(graph.links().iter().any(|l| l.from == added && l.to == surface && l.to_socket == Socket::Param(Param::BaseColor)), "and it into the surface: {:?}", graph.links());
        assert!(!graph.links().iter().any(|l| l.from == colour && l.to == surface), "the old wire is gone");
        assert_eq!(h.editor.insert_into, None, "the wire is no longer lit");
    }
    /// A node dragged onto another stays where it was put, and nothing else
    /// moves: the graph is laid out for the person only when nobody has
    /// laid it out, every node where an importer left it.
    #[test]
    fn a_node_dropped_on_another_does_not_rearrange_the_graph() {
        let mut h = Harness::new();
        let mut graph = h.graph();
        let surface = graph.output();
        let at = graph.node(surface).unwrap().position;
        let mut place = |dx: f32, dy: f32| {
            let mut node = Node::new(NodeKind::Constant(Value::Float(0.5)));
            node.position = GraphVec2 { x: at.x - dx, y: at.y + dy };
            graph.add_node(node)
        };
        let (a, b, c) = (place(480.0, 0.0), place(480.0, 240.0), place(240.0, 480.0));
        h.project.apply(Command::SetGraph { material: h.key, graph: Box::new(graph) }).expect("a graph");
        h.project.end_interaction();
        h.editor.frame_pending = true;
        h.frame(Vec::new());
        h.frame(Vec::new());
        let position = |h: &Harness, id: NodeId| h.graph().node(id).unwrap().position;
        let (was_c, was_surface) = (position(&h, c), position(&h, surface));

        // a, by its header, onto b.
        let header = |h: &Harness, id: NodeId| {
            let rect = h.layout_of(id).header();
            h.screen(rect.center())
        };
        let (from, to) = (header(&h, a), header(&h, b));
        h.drag(from, to, egui::PointerButton::Primary, egui::Modifiers::NONE);
        let (now_a, now_b) = (position(&h, a), position(&h, b));
        assert!((now_a.x - now_b.x).abs() < 30.0 && (now_a.y - now_b.y).abs() < 30.0, "a is on b: {now_a:?} {now_b:?}");
        assert_eq!(position(&h, c), was_c, "a node that was not touched moved");
        assert_eq!(position(&h, surface), was_surface, "the surface moved");
        // And it is shown where it is kept.
        let shown = h.layout_of(c).rect.min;
        assert_eq!((shown.x, shown.y), (was_c.x, was_c.y));
    }
    /// Shift+D sets a copy down a step from its original, on top of it: the
    /// other nodes stay where they were. Any overlap used to count as a
    /// graph nobody had laid out, and a duplicate re-laid all of it.
    #[test]
    fn a_duplicate_does_not_rearrange_the_graph() {
        let mut h = Harness::new();
        let mut graph = h.graph();
        let surface = graph.output();
        let at = graph.node(surface).unwrap().position;
        let mut place = |dx: f32, dy: f32| {
            let mut node = Node::new(NodeKind::Constant(Value::Float(0.5)));
            node.position = GraphVec2 { x: at.x - dx, y: at.y + dy };
            graph.add_node(node)
        };
        let (a, b) = (place(480.0, 0.0), place(240.0, 480.0));
        h.project.apply(Command::SetGraph { material: h.key, graph: Box::new(graph) }).expect("a graph");
        h.project.end_interaction();
        h.editor.frame_pending = true;
        h.frame(Vec::new());
        h.frame(Vec::new());
        let position = |h: &Harness, id: NodeId| h.graph().node(id).unwrap().position;
        let was = (position(&h, a), position(&h, b), position(&h, surface));

        let header = h.screen(h.layout_of(a).header().center());
        h.click(header, egui::Modifiers::NONE);
        h.key(egui::Key::D, egui::Modifiers::SHIFT);
        // Set down where it is: a step from the original.
        h.button(egui::PointerButton::Primary, true, egui::Modifiers::NONE);
        h.button(egui::PointerButton::Primary, false, egui::Modifiers::NONE);
        h.frame(Vec::new());
        h.frame(Vec::new());
        assert_eq!(h.graph().len(), 4, "the copy is there");
        assert_eq!((position(&h, a), position(&h, b), position(&h, surface)), was, "the others moved");
        assert!(!tidy::unplaced(&h.graph()), "no two nodes are at one place");
    }
    /// A setup is several nodes added wired: the blend's coats each on the
    /// Layer of the one under it with the stack's factors, the two-sided's
    /// back on the faces' backs, the composite's colours one over another.
    /// Each compiles, and the layered ones are layered as deep as they say.
    #[test]
    fn a_setup_is_added_wired_and_compiles() {
        use tequila_materials::graph::LAYER_INPUT;
        let add = |setup: browser::Setup| {
            let mut h = Harness::with_browser();
            let mut graph = h.graph();
            assert!(h.editor.browse(browser::Act::Arrange { setup, at: None }, &mut graph, &h.project, &Library::none(), &[]));
            graph.compile().unwrap_or_else(|e| panic!("{setup:?} does not compile: {e:?}"));
            assert!(!tidy::unplaced(&graph), "{setup:?} left two nodes at one place");
            assert_eq!(graph.frames.iter().filter(|f| f.title == setup.title()).count(), 1);
            graph
        };
        let depth = |graph: &MaterialGraph| {
            let mut layer = graph.compile().expect("compiles").layer;
            let mut deep = 0;
            while let Some(next) = layer {
                deep += 1;
                layer = next.shader.layer;
            }
            deep
        };

        let blend = add(browser::Setup::Blend);
        // (The top two, both plastic as they are added, are one surface
        // mixed parameter by parameter: two layers deep until one is made
        // another kind.)
        assert_eq!(depth(&blend), 2, "the coats are layers on one another");
        let stack = blend.nodes().find(|(_, n)| matches!(n.kind, NodeKind::Group(tequila_materials::groups::NodeGroup::BlendStack))).map(|(id, _)| id).expect("the stack");
        let mixes: Vec<NodeId> = blend.nodes().filter(|(_, n)| matches!(n.kind, NodeKind::MixShader { add: false })).map(|(id, _)| id).collect();
        assert_eq!(mixes.len(), 3);
        for mix in &mixes {
            assert!(blend.links().iter().any(|l| l.from == stack && l.to == *mix && l.to_socket == Socket::Input(0)), "a Mix Shader's factor is not the stack's");
        }
        assert!(blend.links().iter().any(|l| l.to == blend.output() && l.to_socket == LAYER_INPUT), "the first coat is on the material's Layer");

        let two = add(browser::Setup::TwoSided);
        let mix = two.nodes().find(|(_, n)| matches!(n.kind, NodeKind::MixShader { .. })).map(|(id, _)| id).unwrap();
        let side = two.links().iter().find(|l| l.to == mix && l.to_socket == Socket::Input(0)).map(|l| (l.from, l.from_socket)).expect("a factor");
        let geometry = two.node(side.0).unwrap();
        let NodeKind::Node(node) = &geometry.kind else { panic!("the factor is not a node's") };
        assert_eq!(node.outputs()[side.1 as usize].name, "Backfacing");

        // The per-part ones: each reads the part's random number, compiles,
        // and is left for the person to wire in.
        for setup in [browser::Setup::RandomColour, browser::Setup::VaryColour, browser::Setup::RandomPlacement] {
            let graph = add(setup);
            assert!(!graph.links().iter().any(|l| l.to == graph.output()), "{setup:?} wired itself into the material");
            let info = graph.nodes().find(|(_, n)| matches!(n.kind, NodeKind::Node(tequila_materials::nodes::ShaderNode::ObjectInfo))).map(|(id, _)| id).expect("an Object Info");
            assert!(graph.links().iter().any(|l| l.from == info && l.from_socket == RANDOM), "{setup:?} does not read the part's number");
        }
        assert_eq!(tequila_materials::nodes::ShaderNode::ObjectInfo.outputs()[RANDOM as usize].name, "Random");
        assert_eq!(tequila_materials::nodes::ShaderNode::TextureCoordinate.outputs()[OBJECT as usize].name, "Object");
        // Wired in, each still compiles: a colour into Color, coordinates
        // into a texture's Vector.
        let mut graph = add(browser::Setup::RandomColour);
        let ramp = graph.nodes().find(|(_, n)| matches!(n.kind, NodeKind::Gradient { .. })).map(|(id, _)| id).unwrap();
        let surface = graph.output();
        graph.connect_output(ramp, 0, surface, Socket::Param(Param::BaseColor));
        let program = graph.compile().expect("a random colour compiles into Color");
        assert!(program.driven.iter().any(|(p, _)| *p == Param::BaseColor));
        let mut graph = add(browser::Setup::RandomPlacement);
        let mapping = graph.nodes().find(|(_, n)| matches!(&n.kind, NodeKind::Node(node) if node.label() == "Mapping")).map(|(id, _)| id).unwrap();
        let noise = graph.add_node(Node::new(NodeKind::Node(tequila_materials::nodes::ShaderNode::from_key("noise_texture").unwrap())));
        let surface = graph.output();
        graph.connect_output(mapping, 0, noise, Socket::Input(0));
        graph.connect_output(noise, 0, surface, Socket::Param(Param::SpecularRoughness));
        graph.compile().expect("random placement compiles into a texture");

        // Stochastic tiling: the group's three sets of coordinates each into
        // an Image's Coordinates, its blends into the Mixes; and wired into
        // a colour, the pictures are read where the group says.
        let mut tiling = add(browser::Setup::StochasticTiling);
        let group = tiling.nodes().find(|(_, n)| matches!(n.kind, NodeKind::Group(tequila_materials::groups::NodeGroup::StochasticTiling))).map(|(id, _)| id).expect("the group");
        let images: Vec<NodeId> = tiling.nodes().filter(|(_, n)| matches!(n.kind, NodeKind::ImageTexture { .. })).map(|(id, _)| id).collect();
        assert_eq!(images.len(), 3);
        for image in &images {
            assert!(tiling.links().iter().any(|l| l.from == group && l.to == *image && l.to_socket == Socket::Input(0)), "an Image is not read where the group says");
        }
        let mixes: Vec<NodeId> = tiling.nodes().filter(|(_, n)| matches!(&n.kind, NodeKind::Node(node) if node.label() == "Mix")).map(|(id, _)| id).collect();
        assert_eq!(mixes.len(), 2);
        for blend in [3u8, 4] {
            assert!(tiling.links().iter().any(|l| l.from == group && l.from_socket == blend && mixes.contains(&l.to) && l.to_socket == Socket::Input(0)), "no Mix's factor is the group's blend {blend}");
        }
        assert!(!tiling.links().iter().any(|l| l.to == tiling.output()), "left for the person to wire in");
        // The last Mix: the one no other Mix reads.
        let mix = *mixes.iter().find(|m| !tiling.links().iter().any(|l| l.from == **m)).expect("a last Mix");
        let surface = tiling.output();
        tiling.connect_output(mix, 0, surface, Socket::Param(Param::BaseColor));
        let program = tiling.compile().expect("stochastic tiling compiles into Color");
        let placed = program.instructions.iter().filter(|i| matches!(i, tequila_materials::graph::Instruction::Sample { at: Some(_), .. })).count();
        assert_eq!(placed, 3, "every reading is placed by the group");

        // What a Max user reaches for. The two that are surfaces go on the
        // material's Layer; the rest are left to wire in, and each wired
        // into what it is for compiles and drives it.
        for setup in [browser::Setup::TopBottom, browser::Setup::Override] {
            let graph = add(setup);
            let mix = graph.links().iter().find(|l| l.to == graph.output() && l.to_socket == LAYER_INPUT).map(|l| l.from).unwrap_or_else(|| panic!("{setup:?} is not on the material's Layer"));
            assert!(matches!(graph.node(mix).map(|n| &n.kind), Some(NodeKind::MixShader { add: false })));
            assert!(graph.links().iter().any(|l| l.to == mix && l.to_socket == Socket::Input(0)), "{setup:?} has nothing to say where its surface shows");
            graph.compile().unwrap_or_else(|e| panic!("{setup:?} does not compile: {e:?}"));
        }
        for (setup, last, param) in [
            (browser::Setup::Falloff, "Front to side", Param::BaseColor),
            (browser::Setup::ColourCorrection, "Gamma", Param::BaseColor),
            (browser::Setup::Curves, "Combine Color", Param::BaseColor),
            (browser::Setup::WornEdges, "Wear", Param::SpecularRoughness),
            (browser::Setup::Dirt, "Dirt", Param::SpecularRoughness),
        ] {
            let mut graph = add(setup);
            assert!(!graph.links().iter().any(|l| l.to == graph.output()), "{setup:?} wired itself into the material");
            let from = graph
                .nodes()
                .find(|(_, n)| n.label.as_deref() == Some(last) || matches!(&n.kind, NodeKind::Node(node) if node.label() == last))
                .map(|(id, _)| id)
                .unwrap_or_else(|| panic!("{setup:?} has no {last}"));
            let surface = graph.output();
            graph.connect_output(from, 0, surface, Socket::Param(param));
            let program = graph.compile().unwrap_or_else(|e| panic!("{setup:?} does not compile: {e:?}"));
            // (A colour corrected is still one colour: folded into the
            // material, with nothing left to drive.)
            let constant = matches!(setup, browser::Setup::ColourCorrection | browser::Setup::Curves);
            assert!(constant || program.driven.iter().any(|(p, _)| *p == param), "{setup:?} does not drive {param:?}");
        }
        // Distorted coordinates place a picture.
        let mut graph = add(browser::Setup::Distort);
        let distorted = graph.nodes().find(|(_, n)| n.label.as_deref() == Some("Distorted")).map(|(id, _)| id).expect("Distorted");
        let image = graph.add_node(Node::new(NodeKind::ImageTexture { texture: tequila_materials::graph::TextureId(1), fallback: Value::Color(Spectrum::splat(0.18)) }));
        let surface = graph.output();
        graph.connect_output(distorted, 0, image, Socket::Input(0));
        graph.connect_output(image, 0, surface, Socket::Param(Param::BaseColor));
        let program = graph.compile().expect("a distorted picture compiles");
        assert!(program.instructions.iter().any(|i| matches!(i, tequila_materials::graph::Instruction::Sample { at: Some(_), .. })), "the picture is not read at the distorted coordinates");

        let composite = add(browser::Setup::Composite);
        assert_eq!(depth(&composite), 0, "colours, not surfaces");
        assert!(!composite.links().iter().any(|l| l.to == composite.output()), "left for the person to wire in");
        assert_eq!(composite.nodes().filter(|(_, n)| matches!(n.kind, NodeKind::Constant(Value::Color(_)))).count(), 4);

        // A material whose Layer is taken keeps it: the setup is added beside.
        let mut h = Harness::with_browser();
        let mut graph = h.graph();
        assert!(h.editor.browse(browser::Act::Arrange { setup: browser::Setup::TwoSided, at: None }, &mut graph, &h.project, &Library::none(), &[]));
        let first = graph.links().iter().find(|l| l.to == graph.output() && l.to_socket == LAYER_INPUT).copied().expect("layered");
        assert!(h.editor.browse(browser::Act::Arrange { setup: browser::Setup::Blend, at: None }, &mut graph, &h.project, &Library::none(), &[]));
        assert_eq!(graph.links().iter().filter(|l| l.to == graph.output() && l.to_socket == LAYER_INPUT).copied().collect::<Vec<_>>(), [first]);
        graph.compile().expect("still compiles");
    }
}
