//! The Add menu: Shift+A, Tab, a right-click or a wire dropped in space.
//!
//! As Blender's: the families down the left, their nodes beside, and typing
//! searches everything at once - including the operations inside a node, so
//! "multiply" finds Math ▸ Multiply, Vector Math ▸ Multiply and Mix ▸
//! Multiply, each added already set to it. Arrow keys move through the
//! results and Enter adds one, so a node can be added without the mouse.

use super::view::{family, Family};
use crate::project::Project;
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{NodeKind, SocketType, Value};
use tequila_materials::nodes::{Setting, ShaderNode};

#[derive(Clone)]
pub struct Entry {
    pub label: String,
    /// Where it lives, for a search result: "Math", "Mix".
    pub parent: Option<&'static str>,
    pub family: Family,
    pub kind: NodeKind,
    /// Found by searching, not listed in its family.
    pub search_only: bool,
}

fn shader(key: &str) -> ShaderNode {
    ShaderNode::from_key(key).unwrap_or_else(|| panic!("no node {key}"))
}

fn set(mut node: ShaderNode, key: &str, value: Setting) -> ShaderNode {
    node.set(key, value);
    node
}

pub fn entries(project: &Project) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut add = |label: &str, kind: NodeKind| {
        out.push(Entry { label: label.into(), parent: None, family: family(&kind), kind, search_only: false });
    };
    // Input.
    for node in ShaderNode::catalog().into_iter().filter(|n| matches!(family(&NodeKind::Node(*n)), Family::Input)) {
        add(node.menu_label(), NodeKind::Node(node));
    }
    add("Value", NodeKind::Constant(Value::Float(0.5)));
    add("RGB", NodeKind::Constant(Value::Color(Spectrum::splat(0.8))));
    add("Vertex Color", NodeKind::VertexColor);
    add("Projection", NodeKind::Mapping2d { mapping: Default::default() });
    // Textures.
    for (id, source) in project.document().textures.iter() {
        add(&format!("Image · {}", source.display_name()), NodeKind::ImageTexture { texture: id, fallback: Value::Color(Spectrum::splat(0.18)) });
        add(&format!("Image Alpha · {}", source.display_name()), NodeKind::ImageAlpha { texture: id, fallback: Value::Float(1.0) });
    }
    for node in ShaderNode::catalog().into_iter().filter(|n| matches!(family(&NodeKind::Node(*n)), Family::Texture)) {
        add(node.menu_label(), NodeKind::Node(node));
    }
    // Tequila's own patterns.
    for procedural in tequila_texture::procedural::Procedural::catalog() {
        add(procedural.label(), NodeKind::Procedural { procedural: Box::new(procedural), seed: 0 });
    }
    // Node groups: whole surfaces made of nodes.
    for &group in tequila_materials::groups::NodeGroup::ALL {
        add(group.label(), NodeKind::Group(group));
    }
    // Colour.
    for node in ShaderNode::catalog().into_iter().filter(|n| matches!(family(&NodeKind::Node(*n)), Family::Color)) {
        let label = if matches!(node, ShaderNode::Mix { .. }) { "Mix Color" } else { node.menu_label() };
        add(label, NodeKind::Node(node));
    }
    // Converter.
    add("Color Ramp", NodeKind::Gradient { gradient: Box::default() });
    for node in ShaderNode::catalog().into_iter().filter(|n| matches!(family(&NodeKind::Node(*n)), Family::Converter)) {
        add(node.menu_label(), NodeKind::Node(node));
    }
    // Vector.
    for node in ShaderNode::catalog().into_iter().filter(|n| matches!(family(&NodeKind::Node(*n)), Family::Vector)) {
        add(node.menu_label(), NodeKind::Node(node));
    }
    add("Bump", NodeKind::Bump { depth: tequila_materials::graph::DEFAULT_BUMP_DEPTH });
    // Surface.
    add(
        "Curvature",
        NodeKind::Curvature {
            mode: tequila_materials::graph::CurvatureMode::Convex,
            radius: tequila_materials::graph::DEFAULT_CURVATURE_RADIUS,
            contrast: 1.0,
        },
    );
    add("Occlusion", NodeKind::Occlusion { radius: tequila_materials::graph::DEFAULT_OCCLUSION_RADIUS, contrast: 1.0 });
    // Mixing surfaces.
    add("Shader", NodeKind::Shader(Box::new(tequila_materials::material_type::MaterialType::Plastic.defaults())));
    add("Mix Shader", NodeKind::MixShader { add: false });
    add("Add Shader", NodeKind::MixShader { add: true });
    add("Bubbles", NodeKind::Bubbles { count: 200, radius: 0.002, variation: 0.5, seed: 0 });
    add("Fuzz", NodeKind::Fuzz { length: 0.002, density: 400.0, thickness: 0.00002, lean: 0.35, curl: 0.3, seed: 0 });
    // Layout.
    add("Reroute", NodeKind::Reroute { ty: SocketType::Color });

    // Search only: the operations inside a node, each a node already set.
    let mut variant = |parent: &'static str, label: &str, kind: NodeKind| {
        out.push(Entry { label: label.into(), parent: Some(parent), family: family(&kind), kind, search_only: true });
    };
    // A group by what it is for: "3d print" finds the layer lines.
    for &group in tequila_materials::groups::NodeGroup::ALL {
        for words in group.keywords().split(", ") {
            variant(group.label(), words, NodeKind::Group(group));
        }
    }
    for op in tequila_texture::cycles::ops::MathOp::ALL {
        variant("Math", op.label(), NodeKind::Node(set(shader("math"), "operation", Setting::Choice(op.cycles()))));
    }
    for op in tequila_texture::cycles::ops::VectorMathOp::ALL {
        variant("Vector Math", op.label(), NodeKind::Node(set(shader("vector_math"), "operation", Setting::Choice(op.cycles()))));
    }
    for mode in tequila_texture::cycles::ops::BlendMode::ALL {
        variant("Mix", mode.label(), NodeKind::Node(set(shader("mix"), "blend_type", Setting::Choice(mode.cycles()))));
    }
    for kind in tequila_texture::cycles::noise::FractalKind::ALL {
        variant("Noise Texture", kind.label(), NodeKind::Node(set(shader("noise_texture"), "type", Setting::Choice(kind.cycles()))));
    }
    for feature in tequila_texture::cycles::voronoi::VoronoiFeature::ALL {
        variant("Voronoi Texture", feature.label(), NodeKind::Node(set(shader("voronoi_texture"), "feature", Setting::Choice(feature.cycles()))));
    }
    variant("Wave Texture", "Rings", NodeKind::Node(set(shader("wave_texture"), "type", Setting::Choice("rings"))));
    variant("Mix", "Mix Float", NodeKind::Node(set(shader("mix"), "data_type", Setting::Choice("float"))));
    variant("Mix", "Mix Vector", NodeKind::Node(set(shader("mix"), "data_type", Setting::Choice("vector"))));
    out
}

/// How well `query` matches `text`: higher is better, `None` for no match.
/// Whole-word starts beat a run of letters anywhere, which beats letters
/// scattered through it, so "mul" puts Multiply above Modulo.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let query: String = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some(0);
    }
    let lower: String = text.chars().flat_map(char::to_lowercase).collect();
    if lower.starts_with(&query) {
        return Some(300 - text.len() as i32);
    }
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| w.starts_with(&query)) {
        return Some(200 - text.len() as i32);
    }
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains(&query) {
        return Some(150 - text.len() as i32);
    }
    // Initials: "vm" for Vector Math.
    let initials: String = words.iter().filter_map(|w| w.chars().next()).collect();
    if initials.starts_with(&query) {
        return Some(120 - text.len() as i32);
    }
    let mut letters = compact.chars();
    if query.chars().all(|q| letters.any(|t| t == q)) {
        return Some(50 - text.len() as i32);
    }
    None
}

pub struct AddMenu {
    /// Where on the canvas the new node goes.
    pub at: egui::Vec2,
    /// Where on screen the menu opened.
    pub screen: egui::Pos2,
    pub query: String,
    /// Wire the new node to this, if the menu came from a dropped wire.
    pub from: Option<super::End>,
    pub focus: bool,
    pub family: Family,
    /// The highlighted result.
    pub highlight: usize,
}

impl AddMenu {
    pub fn new(at: egui::Vec2, screen: egui::Pos2, from: Option<super::End>) -> AddMenu {
        AddMenu { at, screen, query: String::new(), from, focus: true, family: Family::Texture, highlight: 0 }
    }
}

/// What the menu came to.
pub enum Outcome {
    Open,
    Close,
    Add(NodeKind),
}

/// Draw the menu; `fits` says which entries can be wired where it came from.
pub fn show(ctx: &egui::Context, menu: &mut AddMenu, entries: &[Entry], fits: &dyn Fn(&NodeKind) -> bool, bounds: egui::Rect) -> Outcome {
    let searching = !menu.query.trim().is_empty();
    let mut results: Vec<(&Entry, i32)> = if searching {
        let mut found: Vec<(&Entry, i32)> = entries
            .iter()
            .filter(|e| fits(&e.kind))
            .filter_map(|e| {
                let own = score(&menu.query, &e.label);
                let with_parent = e.parent.and_then(|p| score(&menu.query, &format!("{p} {}", e.label)).map(|s| s - 5));
                own.max(with_parent).map(|s| (e, s - if e.search_only { 8 } else { 0 }))
            })
            .collect();
        found.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.label.len().cmp(&b.0.label.len())));
        found.truncate(14);
        found
    } else {
        entries.iter().filter(|e| !e.search_only && e.family == menu.family && fits(&e.kind)).map(|e| (e, 0)).collect()
    };
    if menu.highlight >= results.len() {
        menu.highlight = results.len().saturating_sub(1);
    }

    // Keys first, before the search field can take them.
    let (down, up, enter, escape) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    if escape {
        return Outcome::Close;
    }
    if down && !results.is_empty() {
        menu.highlight = (menu.highlight + 1) % results.len();
    }
    if up && !results.is_empty() {
        menu.highlight = (menu.highlight + results.len() - 1) % results.len();
    }
    if enter {
        return match results.get(menu.highlight) {
            Some((entry, _)) => Outcome::Add(entry.kind.clone()),
            None => Outcome::Close,
        };
    }

    let width = if searching { 300.0 } else { 360.0 };
    let height = 360.0;
    let position = egui::pos2(
        menu.screen.x.min(bounds.right() - width).max(bounds.left()),
        menu.screen.y.min(bounds.bottom() - height).max(bounds.top()),
    );
    let mut outcome = Outcome::Open;
    let dim = crate::theme::dim();
    egui::Area::new(egui::Id::new("graph-add-menu")).fixed_pos(position).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(6.0)).show(ui, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Add").strong());
                let field = ui.add(egui::TextEdit::singleline(&mut menu.query).hint_text("Search nodes…").desired_width(f32::INFINITY));
                if std::mem::take(&mut menu.focus) {
                    field.request_focus();
                }
                if field.changed() {
                    menu.highlight = 0;
                }
            });
            ui.separator();
            if searching {
                egui::ScrollArea::vertical().max_height(height - 50.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for (i, (entry, _)) in results.iter_mut().enumerate() {
                        let chosen = i == menu.highlight;
                        let response = ui
                            .horizontal(|ui| {
                                let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
                                if chosen || response.hovered() {
                                    ui.painter().rect_filled(rect, 4.0, if chosen { crate::theme::p().accent.gamma_multiply(0.35) } else { crate::theme::p().hover });
                                }
                                ui.painter().circle_filled(rect.left_center() + egui::vec2(9.0, 0.0), 4.0, entry.family.tint().gamma_multiply(1.6));
                                let text = match entry.parent {
                                    Some(parent) => format!("{parent} › {}", entry.label),
                                    None => entry.label.clone(),
                                };
                                ui.painter().text(rect.left_center() + egui::vec2(20.0, 0.0), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(13.0), crate::theme::p().text);
                                ui.painter().text(rect.right_center() - egui::vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER, entry.family.label(), egui::FontId::proportional(11.0), dim);
                                response
                            })
                            .inner;
                        if response.hovered() {
                            menu.highlight = i;
                        }
                        if response.clicked() {
                            outcome = Outcome::Add(entry.kind.clone());
                        }
                    }
                    if results.is_empty() {
                        ui.label(egui::RichText::new("Nothing found that fits here").color(dim));
                    }
                });
            } else {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(118.0);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        for family in Family::MENU {
                            let any = entries.iter().any(|e| !e.search_only && e.family == family && fits(&e.kind));
                            let (rect, response) = ui.allocate_exact_size(egui::vec2(118.0, 26.0), egui::Sense::click_and_drag());
                            let chosen = menu.family == family;
                            if chosen {
                                ui.painter().rect_filled(rect, 4.0, crate::theme::p().pressed);
                            } else if response.hovered() {
                                ui.painter().rect_filled(rect, 4.0, crate::theme::p().hover);
                            }
                            ui.painter().rect_filled(egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 7.0), egui::vec2(3.0, 12.0)), 1.5, family.tint().gamma_multiply(1.7));
                            let colour = if any { crate::theme::p().text } else { crate::theme::p().text_faint };
                            ui.painter().text(rect.left_center() + egui::vec2(14.0, 0.0), egui::Align2::LEFT_CENTER, family.label(), egui::FontId::proportional(13.0), colour);
                            ui.painter().text(rect.right_center() - egui::vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, "›", egui::FontId::proportional(13.0), dim);
                            // Hovering a family opens it, as a menu does.
                            if response.hovered() || response.clicked() {
                                menu.family = family;
                                menu.highlight = 0;
                            }
                        }
                    });
                    ui.separator();
                    // Its own column: inside the row above, a scroll area
                    // would lay the nodes out side by side.
                    ui.vertical(|ui| egui::ScrollArea::vertical().id_salt("graph-add-family").max_height(height - 50.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 1.0;
                        for (i, (entry, _)) in results.iter().enumerate() {
                            let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
                            if i == menu.highlight || response.hovered() {
                                ui.painter().rect_filled(rect, 4.0, if i == menu.highlight { crate::theme::p().accent.gamma_multiply(0.35) } else { crate::theme::p().hover });
                            }
                            ui.painter().text(rect.left_center() + egui::vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, &entry.label, egui::FontId::proportional(13.0), crate::theme::p().text);
                            if response.hovered() {
                                menu.highlight = i;
                            }
                            if response.clicked() {
                                outcome = Outcome::Add(entry.kind.clone());
                            }
                        }
                        if results.is_empty() {
                            ui.label(egui::RichText::new("Nothing here fits the wire").color(dim));
                        }
                    }));
                });
            }
        });
    });
    // A click anywhere else closes it.
    if matches!(outcome, Outcome::Open) && ctx.input(|i| i.pointer.any_pressed()) && !super::over_popup(ctx, "graph-add-menu") {
        return Outcome::Close;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ranks_word_starts_first() {
        assert!(score("mul", "Multiply").unwrap() > score("mul", "Smooth Minimum").unwrap_or(i32::MIN));
        assert!(score("vm", "Vector Math").is_some(), "initials find a node");
        assert!(score("grd", "Gradient Texture").is_some(), "letters in order find one");
        assert!(score("xyzq", "Gradient").is_none());
        assert!(score("noise", "Noise Texture").unwrap() > score("noise", "White Noise Texture").unwrap());
    }
}
