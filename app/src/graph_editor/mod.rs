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

mod menu;
mod previews;
mod sidebar;
mod swatch;
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

    pub fn show(&mut self, ctx: &egui::Context, project: &mut Project) {
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
        let exposed = definition.material_type.exposed();
        let original = definition.graph.clone();
        // A graph nobody laid out is shown laid out, but not changed for
        // being looked at: the positions are saved with the first real edit.
        let baseline = if tidy::overlapping(&original, &exposed) {
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
            size: egui::vec2(1240.0, 720.0),
            min_size: egui::vec2(720.0, 420.0),
            resizable: true,
            margin: 0.0,
        };
        let window = crate::windows::show(ctx, spec, &mut open, |window| window.default_size([1240.0, 720.0]).min_width(720.0).min_height(420.0).resizable(true), |ui| {
                egui::SidePanel::right("graph-properties")
                    .resizable(true)
                    .default_width(300.0)
                    .width_range(240.0..=460.0)
                    .show_inside(ui, |ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            ui.add_space(4.0);
                            self.swatch_panel(ui, &graph, project);
                            ui.add_space(4.0);
                            ui.separator();
                            committed |= self.properties(ui, project, &mut graph);
                        });
                    });
                egui::TopBottomPanel::bottom("graph-status").show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        let text = match &self.message {
                            Some(message) => egui::RichText::new(message).color(crate::theme::p().danger),
                            None => egui::RichText::new(format!(
                                "{} nodes · {} selected · {:.0}%   —   Shift A: add · drag a socket: wire · Ctrl right-drag: cut · G: move · M: mute · H: collapse",
                                graph.len(),
                                self.selected.len(),
                                self.zoom * 100.0
                            ))
                            .small()
                            .color(crate::theme::dim()),
                        };
                        ui.label(text);
                    });
                });
                egui::TopBottomPanel::top("graph-toolbar")
                    .frame(egui::Frame::none().fill(crate::theme::p().chrome).inner_margin(egui::Margin::symmetric(8.0, 5.0)))
                    .show_inside(ui, |ui| {
                        self.toolbar(ui, &mut graph, &exposed, &mut committed);
                    });
                egui::CentralPanel::default().frame(egui::Frame::none().fill(ink::canvas())).show_inside(ui, |ui| {
                    committed |= self.canvas(ui, project, &mut graph, &exposed);
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
            match project.apply(Command::SetGraph { material: key, graph: Box::new(graph) }) {
                Ok(()) => self.message = None,
                Err(error) => self.message = Some(explain(&error.to_string())),
            }
            if !dragging || committed {
                project.end_interaction();
            }
        } else if pointer_released {
            project.end_interaction();
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
                self.grab = None;
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
        self.selected = fresh.values().copied().collect();
        self.active = fresh.values().next().copied();
        self.selected_frame = None;
        true
    }

    /// The wire a node at `screen` sits on, if any: one it could go into.
    fn wire_under(&self, graph: &MaterialGraph, layouts: &HashMap<NodeId, NodeLayout>, rect: egui::Rect, screen: egui::Rect, id: NodeId) -> Option<Link> {
        let node = graph.node(id)?;
        let inputs = node.kind.inputs();
        let outputs = node.kind.outputs();
        if inputs.is_empty() || outputs.is_empty() {
            return None;
        }
        graph.links().iter().copied().find(|link| {
            if link.from == id || link.to == id {
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
                    let response = ui.add(egui::Button::new(label).shortcut_text(keys).frame(false).min_size(egui::vec2(180.0, 22.0)));
                    if response.clicked() {
                        action = Some(name);
                    }
                };
                if target.is_some() {
                    item(ui, "Duplicate", "Shift D", "duplicate");
                    item(ui, "Copy", "Ctrl C", "copy");
                    item(ui, "Mute", "M", "mute");
                    item(ui, "Collapse", "H", "collapse");
                    item(ui, "Solo output", "Ctrl Shift click", "solo");
                    item(ui, "Frame selection", "Ctrl J", "frame");
                    ui.separator();
                    item(ui, "Delete, keep wire", "Ctrl X", "dissolve");
                    item(ui, "Delete", "X", "delete");
                } else {
                    item(ui, "Add…", "Shift A", "add");
                    item(ui, "Paste", "Ctrl V", "paste");
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
                self.selected = BTreeSet::from([id]);
                self.active = Some(id);
                true
            }
        }
    }

    /// The bar above the canvas.
    fn toolbar(&mut self, ui: &mut egui::Ui, graph: &mut MaterialGraph, exposed: &[Param], committed: &mut bool) {
        let rect = self.canvas_rect;
        ui.horizontal(|ui| {
            let style = ui.style_mut();
            style.spacing.item_spacing.x = 4.0;
            style.spacing.button_padding = egui::vec2(10.0, 3.0);
            style.spacing.interact_size.y = 24.0;
            let button = |text: &str| egui::Button::new(egui::RichText::new(text).size(12.5)).fill(crate::theme::p().control).rounding(6.0);
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
            ui.toggle_value(&mut self.show_map, egui::RichText::new("Map").size(12.5)).on_hover_text("An overview of the graph in the corner, when it does not fit the view");
            if self.solo_shown.is_some() && ui.add(button("End solo")).on_hover_text("Show the material again").clicked() {
                self.solo_request = Some(None);
                self.solo_shown = None;
            }
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

fn snap(v: f32) -> f32 {
    (v / 12.0).round() * 12.0
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

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::spectrum::Spectrum;
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
    }

    impl Harness {
        fn new() -> Harness {
            let project = project();
            let key = project.document().materials.iter().next().map(|(k, _)| k).expect("a material");
            let mut editor = GraphEditor::default();
            editor.open_for(key);
            let mut h = Harness { ctx: egui::Context::default(), editor, project, key, pointer: egui::pos2(10.0, 10.0), time: 0.0 };
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
            let _ = self.ctx.run(input, |ctx| editor.show(ctx, project));
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
}
