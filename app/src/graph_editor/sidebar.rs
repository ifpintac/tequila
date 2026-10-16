//! The sidebar: the material rendered large, then the selected node's
//! properties at full size - every setting and value the node body shows in
//! miniature, and the editors for what has no room there (a pattern's
//! parameters, a ramp's stops, a projection).

use super::view::{self, Inline, SettingField, SettingRef, Target};
use super::GraphEditor;
use crate::project::Project;
use tequila_materials::graph::{Channel, Link, MaterialGraph, NodeKind, TextureId, Value};
use tequila_materials::material_type::Param;
use tequila_render_cpu::PreviewShape;

impl GraphEditor {
    /// The material swatch, at the top of the sidebar.
    pub(super) fn swatch_panel(&mut self, ui: &mut egui::Ui, graph: &MaterialGraph, project: &Project) {
        let scene = project.scene();
        // A ball the size of the model, so a pattern sized for the part shows
        // on it as it will on the part: half its largest dimension.
        let radius = {
            let b = scene.bounds();
            let size = b.max - b.min;
            (size.x.max(size.y).max(size.z) * 0.5).clamp(0.005, 50.0)
        };
        self.swatch.update(ui.ctx(), graph, &scene.textures, radius);
        let side = ui.available_width().min(300.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), side), egui::Sense::hover());
        let square = egui::Rect::from_center_size(rect.center(), egui::vec2(side, side));
        ui.painter().rect_filled(square, 8.0, crate::theme::p().field);
        if let Some(texture) = self.swatch.texture() {
            ui.painter().add(egui::Shape::image(texture.id(), square, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE));
            // Rounded to match its frame.
            ui.painter().rect_stroke(square, 8.0, egui::Stroke::new(1.0_f32, crate::theme::p().line));
        } else {
            ui.painter().text(square.center(), egui::Align2::CENTER_CENTER, "Rendering…", egui::FontId::proportional(12.0), crate::theme::dim());
        }
        if !self.swatch.settled() {
            // A small working mark in the corner, not a spinner over the picture.
            let t = ui.input(|i| i.time) as f32;
            let centre = square.right_top() + egui::vec2(-14.0, 14.0);
            for k in 0..3 {
                let phase = (t * 3.0 - k as f32 * 0.6).sin() * 0.5 + 0.5;
                ui.painter().circle_filled(centre + egui::vec2(-(k as f32) * 8.0, 0.0), 2.5, crate::theme::p().text.gamma_multiply((80.0 + 150.0 * phase) / 255.0));
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Shape").small().color(crate::theme::dim()));
            ui.selectable_value(&mut self.swatch.shape, PreviewShape::Sphere, "Sphere");
            ui.selectable_value(&mut self.swatch.shape, PreviewShape::Slab, "Slab");
        });
    }

    /// The selected node's properties. True for a finished edit.
    pub(super) fn properties(&mut self, ui: &mut egui::Ui, project: &Project, graph: &mut MaterialGraph) -> bool {
        if let (None, Some(index)) = (self.active, self.selected_frame.filter(|&i| i < graph.frames.len())) {
            crate::theme::section(ui, "Frame");
            let mut committed = false;
            let mut title = graph.frames[index].title.clone();
            if ui.add(egui::TextEdit::singleline(&mut title).desired_width(f32::INFINITY)).changed() {
                graph.frames[index].title = title;
            }
            ui.label(egui::RichText::new("Drag the title to move the frame and everything in it; drag the corner to resize.").small().color(crate::theme::dim()));
            if ui.button("Remove frame").on_hover_text("The nodes stay where they are").clicked() {
                graph.frames.remove(index);
                self.selected_frame = None;
                committed = true;
            }
            return committed;
        }
        let Some(id) = self.active.filter(|id| graph.node(*id).is_some()) else {
            self.shortcuts(ui);
            return false;
        };
        let node = graph.node(id).cloned().expect("checked above");
        let mut committed = false;
        crate::theme::section(ui, &view::family(&node.kind).label().to_string());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(view::title(&node)).strong().size(15.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_solo = id != graph.output() && (0..node.kind.outputs().len() as u8).any(|o| graph.soloed_at(id, o).is_some());
                if can_solo {
                    let soloed = project.solo().is_some_and(|(_, n, _)| n == id);
                    if ui
                        .add(egui::SelectableLabel::new(soloed, "Solo"))
                        .on_hover_text("Show only this node's output on the model, unlit (Ctrl+Shift+click a node). Click again, or close the editor, to stop.")
                        .clicked()
                    {
                        self.solo_request = Some((!soloed).then_some((id, 0)));
                        self.solo_shown = (!soloed).then_some((id, 0));
                    }
                }
            });
        });
        let mut label = node.label.clone().unwrap_or_default();
        let field = ui.add(egui::TextEdit::singleline(&mut label).hint_text("Name (optional)").desired_width(f32::INFINITY));
        if std::mem::take(&mut self.focus_label) {
            field.request_focus();
        }
        if field.lost_focus() && node.label.clone().unwrap_or_default() != label {
            if let Some(node) = graph.node_mut(id) {
                node.label = (!label.trim().is_empty()).then(|| label.trim().to_string());
                committed = true;
            }
        }
        ui.horizontal(|ui| {
            let mut muted = node.muted;
            if id != graph.output() && ui.add(crate::field::Checkbox::new(&mut muted, "Muted")).on_hover_text("Pass the input straight through (M)").changed() {
                if let Some(node) = graph.node_mut(id) {
                    node.muted = muted;
                    committed = true;
                }
            }
            let mut collapsed = node.collapsed;
            if ui.add(crate::field::Checkbox::new(&mut collapsed, "Collapsed")).on_hover_text("Show the header alone (H)").changed() {
                if let Some(node) = graph.node_mut(id) {
                    node.collapsed = collapsed;
                    committed = true;
                }
            }
        });

        // Blender nodes: their settings, then their unwired inputs.
        if let NodeKind::Node(shader) = node.kind {
            for (i, spec) in shader.settings().iter().enumerate() {
                let setting = SettingRef::Shader(i);
                match view::setting_field(&node, setting) {
                    Some(SettingField::Dropdown { current }) => {
                        let mut chosen = None;
                        egui::ComboBox::new(("graph-setting", id.0, i), spec.label).selected_text(current).show_ui(ui, |ui| {
                            for (key, text) in &spec.options {
                                if ui.selectable_label(text == &current, *text).clicked() {
                                    chosen = Some(*key);
                                }
                            }
                        });
                        if let Some(key) = chosen {
                            committed |= view::choose(graph, id, setting, key);
                        }
                    }
                    Some(SettingField::Number { label, int, range }) => {
                        let target = Target::Setting { node: id, setting };
                        if let Some(Value::Float(mut v)) = view::read(graph, target) {
                            let mut drag = egui::DragValue::new(&mut v).speed(if int { 0.1 } else { 0.01 });
                            if let Some((lo, hi)) = range {
                                drag = drag.range(lo..=hi);
                            }
                            if int {
                                drag = drag.fixed_decimals(0);
                            }
                            if ui.horizontal(|ui| { let r = ui.add(drag); ui.label(label); r }).inner.changed() {
                                view::write(graph, target, Value::Float(v));
                            }
                        }
                    }
                    Some(SettingField::Toggle { label, on }) => {
                        let mut on = on;
                        if ui.add(crate::field::Checkbox::new(&mut on, label)).changed() {
                            committed |= view::write(graph, Target::Setting { node: id, setting }, Value::Bool(on));
                        }
                    }
                    None => {}
                }
            }
        }

        let mut kind = node.kind.clone();
        let changed = ui.push_id(("graph-node", id.0), |ui| legacy_controls(ui, project, &mut kind)).inner;
        if changed {
            if let Some(node) = graph.node_mut(id) {
                node.kind = kind;
            }
        }

        // Values for inputs nothing is wired into, at full size.
        if !matches!(node.kind, NodeKind::Surface(_)) {
            let wired = view::connected(graph);
            let open: Vec<_> = view::visible_inputs(&node, id, &wired, &[])
                .into_iter()
                .filter(|(_, socket, _, _)| !graph.links().iter().any(|l: &Link| l.to == id && l.to_socket == *socket))
                .collect();
            let editable: Vec<_> = open.into_iter().filter(|(index, socket, ty, _)| view::inline_of(&node, *index, *socket, *ty) != Inline::Label).collect();
            if !editable.is_empty() {
                crate::theme::section(ui, "Inputs");
                for (index, socket, ty, label) in editable {
                    let target = Target::Input { node: id, index, component: None };
                    let Some(mut value) = view::read(graph, target) else { continue };
                    let range = match view::inline_of(&node, index, socket, ty) {
                        Inline::Number { range } => range,
                        _ => None,
                    };
                    if ui.push_id(("graph-input", id.0, index), |ui| value_editor(ui, label, &mut value, range)).inner {
                        view::write(graph, target, value);
                    }
                }
            }
        }

        if id != graph.output() {
            ui.add_space(10.0);
            if ui.button("Delete node").on_hover_text("X or Delete").clicked() {
                graph.remove_node(id);
                self.selected.remove(&id);
                self.active = None;
                committed = true;
            }
        }
        committed
    }

    /// With nothing selected: how the editor is driven.
    fn shortcuts(&self, ui: &mut egui::Ui) {
        crate::theme::section(ui, "Getting around");
        // As the Shortcuts window draws them: what it does, then the keys
        // as key caps.
        let rows: [(&str, &str); 18] = [
            ("Add a node", "Shift + A / Tab"),
            ("Wire; drop in space to add", "Drag a socket"),
            ("Cut wires", "Ctrl + Right drag"),
            ("Box select", "Drag"),
            ("Add to the selection", "Shift + Click"),
            ("Move the selection", "G"),
            ("Duplicate", "Shift + D"),
            ("Copy / paste", "Ctrl + C / Ctrl + V"),
            ("Delete", "X / Delete"),
            ("Delete, keeping the wire", "Ctrl + X"),
            ("Mute", "M"),
            ("Collapse", "H"),
            ("Frame the selection", "Ctrl + J"),
            ("Solo a node's output", "Ctrl + Shift + Click"),
            ("Select all / none", "A / Alt + A"),
            ("Fit the graph", "Home"),
            ("Pan", "Middle drag / Alt + Drag"),
            ("Zoom", "Scroll"),
        ];
        let p = crate::theme::p();
        egui::Grid::new("graph-shortcuts").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
            for (what, keys) in rows {
                ui.label(egui::RichText::new(what).size(12.0).color(p.text_dim));
                crate::theme::key_caps(ui, keys);
                ui.end_row();
            }
        });
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("The Surface node is the material: wire textures, patterns and colours into its inputs. Drop a node on a wire to put it in between.")
                .small()
                .color(crate::theme::dim()),
        );
    }
}

/// The editors for Tequila's own node kinds, whose settings have no room in
/// the node body. True when the kind changed.
fn legacy_controls(ui: &mut egui::Ui, project: &Project, kind: &mut NodeKind) -> bool {
    match kind {
        NodeKind::Reroute { .. } => {
            hint(ui, "Passes its input straight through, to route a long wire tidily. Costs nothing to render.");
            false
        }
        NodeKind::Surface(_) => {
            hint(ui, "The surface's values are in its body and in the Material tab. Wire nodes into its inputs here.");
            false
        }
        NodeKind::Constant(value) => value_editor(ui, "Value", value, None),
        NodeKind::Mapping2d { mapping } => match crate::material_tab::mapping_controls(ui, Param::BaseColor, *mapping) {
            Some(next) => {
                *mapping = next;
                true
            }
            None => false,
        },
        NodeKind::Gradient { gradient } => match crate::material_tab::gradient_controls(ui, Param::BaseColor, gradient) {
            Some(next) => {
                **gradient = next;
                true
            }
            None => false,
        },
        NodeKind::Procedural { procedural, seed } => {
            let mut changed = false;
            if let Some(next) = crate::material_tab::procedural_controls(ui, Param::BaseColor, **procedural) {
                **procedural = next;
                changed = true;
            }
            ui.horizontal(|ui| {
                ui.label("Seed");
                changed |= ui.add(egui::DragValue::new(seed)).changed();
            });
            changed
        }
        NodeKind::ImageTexture { texture, fallback } | NodeKind::ImageAlpha { texture, fallback } => {
            let mut changed = texture_picker(ui, project, texture);
            changed |= value_editor(ui, "Fallback", fallback, None);
            changed
        }
        NodeKind::Remap { from, to } => {
            let mut changed = false;
            for (label, v) in [("From", from), ("To", to)] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    changed |= ui.add(egui::DragValue::new(&mut v.x).speed(0.01)).changed();
                    changed |= ui.add(egui::DragValue::new(&mut v.y).speed(0.01)).changed();
                });
            }
            changed
        }
        NodeKind::Clamp { low, high } => {
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("Between");
                changed |= ui.add(egui::DragValue::new(low).speed(0.01)).changed();
                changed |= ui.add(egui::DragValue::new(high).speed(0.01)).changed();
            });
            changed
        }
        NodeKind::Channel { channel } => {
            let mut changed = false;
            ui.horizontal(|ui| {
                for c in [Channel::Red, Channel::Green, Channel::Blue] {
                    changed |= ui.selectable_value(channel, c, c.label()).changed();
                }
            });
            changed
        }
        NodeKind::Bump { depth } => {
            // In millimetres, for a scene in metres: a bump is a fraction of
            // a millimetre, and six decimal places of metres is unreadable.
            let mut millimetres = *depth * 1000.0;
            let changed = ui
                .horizontal(|ui| {
                    ui.label("Depth");
                    ui.add(egui::DragValue::new(&mut millimetres).speed(0.01).range(0.0..=50.0).suffix(" mm")).changed()
                })
                .inner;
            hint(ui, "How deep the full height is. Wire a pattern or a height map into Height, and this into the surface's Bump.");
            if changed {
                *depth = millimetres / 1000.0;
            }
            changed
        }
        NodeKind::Bubbles { count, radius, variation, seed } => {
            let mut changed = false;
            changed |= ui.add(crate::field::Slider::new(count, 1..=5000).text("How many").logarithmic(true)).changed();
            let mut millimetres = *radius * 1000.0;
            ui.horizontal(|ui| {
                ui.label("Size");
                if ui.add(egui::DragValue::new(&mut millimetres).speed(0.05).range(0.05..=200.0).suffix(" mm")).changed() {
                    *radius = millimetres / 1000.0;
                    changed = true;
                }
            });
            changed |= ui.add(crate::field::Slider::new(variation, 0.0..=1.0).text("Size variation")).changed();
            ui.horizontal(|ui| {
                ui.label("Seed");
                changed |= ui.add(egui::DragValue::new(seed)).changed();
            });
            hint(ui, "Air bubbles inside the part, which must be closed and transmissive: glass, liquid, clear plastic. Needs no wires; being in the graph is enough.");
            changed
        }
        NodeKind::Fuzz { length, density, thickness, lean, curl, seed } => {
            let mut changed = false;
            let mut millimetres = *length * 1000.0;
            ui.horizontal(|ui| {
                ui.label("Length");
                if ui.add(egui::DragValue::new(&mut millimetres).speed(0.02).range(0.05..=50.0).suffix(" mm")).changed() {
                    *length = millimetres / 1000.0;
                    changed = true;
                }
            });
            changed |= ui.add(crate::field::Slider::new(density, 1.0..=5000.0).text("Fibres per cm²").logarithmic(true)).changed();
            let mut microns = *thickness * 1.0e6;
            ui.horizontal(|ui| {
                ui.label("Thickness");
                if ui.add(egui::DragValue::new(&mut microns).speed(0.5).range(2.0..=500.0).suffix(" µm")).changed() {
                    *thickness = microns / 1.0e6;
                    changed = true;
                }
            });
            changed |= ui.add(crate::field::Slider::new(lean, 0.0..=1.0).text("Lean")).changed();
            changed |= ui.add(crate::field::Slider::new(curl, 0.0..=1.0).text("Curl")).changed();
            ui.horizontal(|ui| {
                ui.label("Seed");
                changed |= ui.add(egui::DragValue::new(seed)).changed();
            });
            hint(ui, "Real fibres grown out of the surface in its own material: a peach's skin, felt, a tennis ball, a towel. Needs no wires; being in the graph is enough. Dense, long fuzz on a large part is a lot of geometry - the count is capped.");
            changed
        }
        NodeKind::VolumeMap { .. } => {
            hint(ui, "A density grid from an OpenVDB file, fitted to the part's bounding box. Wire it to a medium's Density. Load one from the Material tab.");
            false
        }
        NodeKind::VertexColor => {
            hint(ui, "The colour the mesh carries at each vertex, blended across its triangles. White where the model has none.");
            false
        }
        NodeKind::Occlusion { radius, contrast } => {
            let mut changed = false;
            let mut millimetres = *radius * 1000.0;
            ui.horizontal(|ui| {
                ui.label("Reach");
                if ui.add(egui::DragValue::new(&mut millimetres).speed(0.1).range(0.1..=2000.0).suffix(" mm")).changed() {
                    *radius = millimetres / 1000.0;
                    changed = true;
                }
            });
            changed |= ui.add(crate::field::Slider::new(contrast, 0.2..=5.0).text("Contrast").logarithmic(true)).changed();
            hint(ui, "Measured on the part itself, once per vertex. 0 is open, 1 enclosed.");
            changed
        }
        NodeKind::Curvature { mode, radius, contrast } => {
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("Mask");
                for m in tequila_materials::graph::CurvatureMode::ALL {
                    changed |= ui.selectable_value(mode, m, m.label()).changed();
                }
            });
            let mut millimetres = *radius * 1000.0;
            ui.horizontal(|ui| {
                ui.label("Radius");
                if ui.add(egui::DragValue::new(&mut millimetres).speed(0.05).range(0.0..=500.0).suffix(" mm")).changed() {
                    *radius = millimetres / 1000.0;
                    changed = true;
                }
            });
            changed |= ui.add(crate::field::Slider::new(contrast, 0.2..=8.0).text("Contrast")).changed();
            hint(ui, "Wear on edges: wire this into a Mix's factor between the paint and the metal beneath. Shows in renders; the Solid view does not measure curvature.");
            changed
        }
        NodeKind::Mix | NodeKind::Multiply | NodeKind::Add | NodeKind::Invert | NodeKind::Node(_) => false,
    }
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().color(crate::theme::dim()));
}

/// Edit a graph value in place. True when it changed.
pub(super) fn value_editor(ui: &mut egui::Ui, label: &str, value: &mut Value, range: Option<(f32, f32)>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        match value {
            Value::Float(v) => {
                changed = match range {
                    Some((lo, hi)) => ui.add(crate::field::Slider::new(v, lo..=hi).clamping(egui::SliderClamping::Never)).changed(),
                    None => ui.add(egui::DragValue::new(v).speed(0.01)).changed(),
                };
            }
            Value::Color(c) => {
                // Values as the Material tab shows them, so a colour reads the
                // same in both places.
                if crate::color_picker::edit(ui, c).changed() {
                    changed = true;
                }
            }
            Value::Vector(v) => {
                changed |= ui.add(egui::DragValue::new(&mut v.x).speed(0.01).prefix("X ")).changed();
                changed |= ui.add(egui::DragValue::new(&mut v.y).speed(0.01).prefix("Y ")).changed();
                changed |= ui.add(egui::DragValue::new(&mut v.z).speed(0.01).prefix("Z ")).changed();
            }
            Value::Bool(b) => {
                changed = ui.add(crate::field::Checkbox::new(b, "")).changed();
            }
        }
        ui.label(label);
    });
    changed
}

fn texture_picker(ui: &mut egui::Ui, project: &Project, texture: &mut TextureId) -> bool {
    let textures = &project.document().textures;
    let current = textures.get(*texture).map(|s| s.display_name()).unwrap_or_else(|| "missing".into());
    let mut changed = false;
    egui::ComboBox::from_label("Image").selected_text(current).show_ui(ui, |ui| {
        for (id, source) in textures.iter() {
            changed |= ui.selectable_value(texture, id, source.display_name()).changed();
        }
    });
    changed
}
