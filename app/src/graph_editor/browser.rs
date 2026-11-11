//! The Material/Map browser, down the editor's left side.
//!
//! As the Slate Material Editor of 3ds Max lays its own out, and V-Ray's
//! lists follow: Materials, Maps and the scene's materials in one list with
//! a search field over it, and a row is dragged into the graph or
//! double-clicked to be added to it. Somebody coming from there looks to
//! the left for what can be made, before they know that Shift+A opens a
//! menu of the same things. It is the Add menu's list, grouped their way,
//! and the editor's toolbar or T hides it.
//!
//! What the groups hold, in Tequila's own terms:
//!
//! - **Materials** are Shader nodes, one for each kind of material: another
//!   surface to mix with the material's own, as a blend material's coats.
//! - **Maps** are the textures and patterns: pictures, noises, weaves.
//! - **Utilities** are the rest of the nodes, by their families.
//! - **Scene materials** are the document's materials: a click opens that
//!   one's graph here, without going back to the Material tab for it.

use super::menu::{score, Entry};
use super::view::Family;
use tequila_materials::graph::NodeKind;
use tequila_materials::material_type::{Category, MaterialType};
use tequila_scene::tree::MaterialKey;

/// The browser's groups, in the order they are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Materials,
    Maps,
    Utilities,
    Scene,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::Materials => "Materials",
            Group::Maps => "Maps",
            Group::Utilities => "Utilities",
            Group::Scene => "Scene materials",
        }
    }

    /// What its rows are, under the group's name when it is open.
    fn about(self) -> &'static str {
        match self {
            Group::Materials => "Another surface to mix with this one: wire it into a Mix Shader, and that into Layer.",
            Group::Maps => "Textures and patterns, for a colour, a roughness or a bump.",
            Group::Utilities => "Values, colour and number work, vectors, and what the part itself gives.",
            Group::Scene => "Click one to edit its graph here.",
        }
    }

    /// Open the first time the browser is seen: what is reached for most.
    fn starts_open(self) -> bool {
        matches!(self, Group::Maps | Group::Scene)
    }
}

/// What a row adds or opens.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A node, and the name it is given when it is not the node's own.
    Node(NodeKind, Option<String>),
    Material(MaterialKey),
}

#[derive(Clone, Debug)]
pub struct Row {
    pub label: String,
    /// The node's family, for its colour; a scene material has none.
    pub family: Option<Family>,
    pub item: Item,
    /// The material the editor is open on.
    pub current: bool,
}

/// A run of rows under one heading of a group.
#[derive(Clone, Debug)]
pub struct Section {
    pub group: Group,
    pub heading: Option<&'static str>,
    pub rows: Vec<Row>,
}

fn category_label(category: Category) -> &'static str {
    match category {
        Category::Basic => "Basic",
        Category::Advanced => "Advanced",
        Category::Special => "Special",
        Category::Light => "Light",
    }
}

/// Everything the browser lists, or what of it `query` finds.
///
/// `entries` are the Add menu's; `materials` the document's, by name. With
/// a query the operations inside a node are found too ("multiply" finds
/// Math › Multiply), best match first within each heading, and a heading
/// with nothing found is left out.
pub fn sections(entries: &[Entry], materials: &[(MaterialKey, String)], current: Option<MaterialKey>, query: &str) -> Vec<Section> {
    let searching = !query.trim().is_empty();
    let mut out: Vec<Section> = Vec::new();
    let mut push = |group: Group, heading: Option<&'static str>, mut rows: Vec<(Row, i32)>| {
        if searching {
            rows.sort_by(|a, b| b.1.cmp(&a.1));
        }
        if !rows.is_empty() {
            out.push(Section { group, heading, rows: rows.into_iter().map(|(row, _)| row).collect() });
        }
    };
    // The Add menu shows its fourteen best; a list shows all it finds, so
    // letters merely in order somewhere in a name ("noise" in Voronoi
    // Texture › N-sphere radius) are not a find here.
    let found = |text: &str| if searching { score(query, text).filter(|s| *s >= LOOSE) } else { Some(0) };

    // Materials: a Shader node of each kind, named for it.
    for category in [Category::Basic, Category::Advanced, Category::Special, Category::Light] {
        let rows = MaterialType::ALL
            .into_iter()
            .filter(|kind| kind.category() == category)
            .filter_map(|kind| {
                let label = kind.label().to_string();
                let score = found(&label)?;
                let item = Item::Node(NodeKind::Shader(Box::new(kind.defaults())), Some(label.clone()));
                Some((Row { label, family: Some(Family::Surface), item, current: false }, score))
            })
            .collect();
        push(Group::Materials, Some(category_label(category)), rows);
    }

    // Maps, then the rest, each family under its own heading.
    let families = |group: Group| -> &'static [Family] {
        match group {
            Group::Maps => &[Family::Texture, Family::Pattern],
            _ => &[Family::Color, Family::Converter, Family::Vector, Family::Input, Family::Surface, Family::Layout],
        }
    };
    for group in [Group::Maps, Group::Utilities] {
        for &family in families(group) {
            let rows = entries
                .iter()
                .filter(|entry| entry.family == family)
                // The plain Shader is the Materials group, a kind at a time.
                .filter(|entry| !matches!(entry.kind, NodeKind::Shader(_)))
                .filter(|entry| searching || !entry.search_only)
                .filter_map(|entry| {
                    let label = match entry.parent {
                        Some(parent) => format!("{parent} › {}", entry.label),
                        None => entry.label.clone(),
                    };
                    let score = found(&label)? - if entry.search_only { 8 } else { 0 };
                    Some((Row { label, family: Some(family), item: Item::Node(entry.kind.clone(), None), current: false }, score))
                })
                .collect();
            push(group, Some(family.label()), rows);
        }
    }

    let rows = materials
        .iter()
        .filter_map(|(key, name)| {
            let score = found(name)?;
            Some((Row { label: name.clone(), family: None, item: Item::Material(*key), current: current == Some(*key) }, score))
        })
        .collect();
    push(Group::Scene, None, rows);
    out
}

/// What was asked of the browser this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    None,
    /// Add a node: where it was dropped on screen, or `None` for a
    /// double-click, which puts it in the middle of the view.
    Add { kind: NodeKind, label: Option<String>, at: Option<egui::Pos2> },
    Open(MaterialKey),
}

#[derive(Default)]
pub struct Browser {
    pub query: String,
    /// The row being dragged out, by its name.
    drag: Option<(String, NodeKind, Option<String>)>,
    /// Where each row was drawn, by its name, for a test's pointer.
    #[cfg(test)]
    pub drawn: Vec<(String, egui::Rect)>,
}

const ROW: f32 = 22.0;

/// Above the Add menu's score for letters scattered through a name, below
/// its scores for a word's start, a run of letters and initials.
const LOOSE: i32 = 68;

impl Browser {
    /// Draw the browser; `canvas` is where a dragged row may be dropped.
    pub fn show(&mut self, ui: &mut egui::Ui, sections: &[Section], canvas: egui::Rect) -> Act {
        let p = crate::theme::p();
        let mut act = Act::None;
        #[cfg(test)]
        self.drawn.clear();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let clear = !self.query.is_empty();
            let room = ui.available_width() - if clear { 26.0 } else { 0.0 };
            ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Search materials and maps…").desired_width(room));
            if clear && ui.add(egui::Button::new("×").frame(false)).on_hover_text("Clear the search").clicked() {
                self.query.clear();
            }
        });
        ui.add_space(2.0);
        let searching = !self.query.trim().is_empty();
        // A drag on a row takes the row out; it does not scroll the list.
        egui::ScrollArea::vertical().id_salt("graph-browser-list").auto_shrink([false, false]).drag_to_scroll(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            if sections.is_empty() {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Nothing by that name").size(12.0).color(p.text_dim));
            }
            for group in [Group::Materials, Group::Maps, Group::Utilities, Group::Scene] {
                let own: Vec<&Section> = sections.iter().filter(|s| s.group == group).collect();
                if own.is_empty() {
                    continue;
                }
                let mut rows = |ui: &mut egui::Ui| {
                    for section in &own {
                        if let Some(heading) = section.heading {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(heading.to_uppercase()).font(crate::theme::strong(10.0)).color(p.text_faint).extra_letter_spacing(0.6));
                        }
                        for row in &section.rows {
                            self.row(ui, row, &mut act);
                        }
                    }
                };
                if searching {
                    // Everything found, in the open: nothing to unfold.
                    crate::theme::section(ui, group.label());
                    rows(ui);
                } else {
                    crate::field::Fold::new(group.label()).id_salt(("graph-browser", group.label())).default_open(group.starts_open()).show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(group.about()).size(11.5).color(p.text_dim)).wrap());
                        rows(ui);
                    });
                }
            }
            ui.add_space(8.0);
        });

        // The row being dragged follows the pointer, and is let go over the
        // canvas to be added there.
        if let Some((name, kind, label)) = self.drag.clone() {
            let (pointer, down) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()));
            if down {
                if let Some(at) = pointer {
                    let over = canvas.contains(at);
                    ui.ctx().set_cursor_icon(if over { egui::CursorIcon::Grabbing } else { egui::CursorIcon::NoDrop });
                    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("graph-browser-drag")));
                    let galley = painter.layout_no_wrap(name, egui::FontId::proportional(12.5), p.text);
                    let pill = egui::Rect::from_min_size(at + egui::vec2(12.0, 10.0), galley.size() + egui::vec2(16.0, 8.0));
                    painter.rect(pill, 6.0, p.card, egui::Stroke::new(1.0_f32, if over { p.accent } else { p.line_strong }));
                    painter.galley(pill.min + egui::vec2(8.0, 4.0), galley, p.text);
                }
            } else {
                self.drag = None;
                if let Some(at) = pointer.filter(|at| canvas.contains(*at)) {
                    act = Act::Add { kind, label, at: Some(at) };
                }
            }
        }
        act
    }

    fn row(&mut self, ui: &mut egui::Ui, row: &Row, act: &mut Act) {
        let p = crate::theme::p();
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click_and_drag());
        #[cfg(test)]
        self.drawn.push((row.label.clone(), rect));
        let dragged = self.drag.as_ref().is_some_and(|(name, _, _)| *name == row.label);
        if row.current {
            ui.painter().rect_filled(rect, 4.0, p.accent.gamma_multiply(0.22));
        } else if response.hovered() || dragged {
            ui.painter().rect_filled(rect, 4.0, p.hover);
        }
        let mark = rect.left_center() + egui::vec2(9.0, 0.0);
        match row.family {
            Some(family) => {
                ui.painter().circle_filled(mark, 4.0, family.tint().gamma_multiply(1.6));
            }
            // A material: a ball, filled for the one being edited.
            None if row.current => {
                ui.painter().circle_filled(mark, 4.5, p.accent);
            }
            None => {
                ui.painter().circle_stroke(mark, 4.0, egui::Stroke::new(1.2_f32, p.text_dim));
            }
        }
        let text = rect.left_center() + egui::vec2(20.0, 0.0);
        ui.painter().with_clip_rect(rect.shrink2(egui::vec2(4.0, 0.0))).text(text, egui::Align2::LEFT_CENTER, &row.label, egui::FontId::proportional(12.5), p.text);
        match &row.item {
            Item::Node(kind, label) => {
                if response.drag_started() {
                    self.drag = Some((row.label.clone(), kind.clone(), label.clone()));
                }
                if response.double_clicked() {
                    *act = Act::Add { kind: kind.clone(), label: label.clone(), at: None };
                }
                response.on_hover_text("Drag it into the graph, or double-click to add it");
            }
            Item::Material(key) => {
                if response.clicked() && !row.current {
                    *act = Act::Open(*key);
                }
                response.on_hover_text(if row.current { "The material this graph is" } else { "Edit this material's graph" });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<Entry> {
        let project = crate::project::Project::with_document(
            tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2),
            std::sync::Arc::new(tequila_scene::demo::studio_environment()),
        );
        super::super::menu::entries(&project)
    }

    fn labels(sections: &[Section], group: Group) -> Vec<String> {
        sections.iter().filter(|s| s.group == group).flat_map(|s| s.rows.iter().map(|r| r.label.clone())).collect()
    }

    /// The browser is the Add menu's list, grouped as a Max user looks for
    /// it: every node the menu lists is in Maps or Utilities once, the
    /// Materials are a Shader of each kind, and the scene's are last.
    #[test]
    fn the_browser_lists_what_the_add_menu_does() {
        let entries = entries();
        let materials = [(MaterialKey(1), "Shell".to_string()), (MaterialKey(2), "Felt Grey".to_string())];
        let sections = sections(&entries, &materials, Some(MaterialKey(2)), "");
        let groups: Vec<Group> = sections.iter().map(|s| s.group).collect();
        let mut order = groups.clone();
        order.dedup();
        assert_eq!(order, [Group::Materials, Group::Maps, Group::Utilities, Group::Scene]);

        let listed: Vec<String> = [labels(&sections, Group::Maps), labels(&sections, Group::Utilities)].concat();
        for entry in entries.iter().filter(|e| !e.search_only && !matches!(e.kind, NodeKind::Shader(_))) {
            assert_eq!(listed.iter().filter(|l| **l == entry.label).count(), 1, "{} is listed once", entry.label);
        }
        assert!(labels(&sections, Group::Maps).iter().any(|l| l == "Noise Texture"));
        assert!(!labels(&sections, Group::Maps).iter().any(|l| l == "Math"), "Math is not a map");
        assert!(labels(&sections, Group::Utilities).iter().any(|l| l == "Math"));

        let kinds = labels(&sections, Group::Materials);
        assert_eq!(kinds.len(), MaterialType::ALL.len());
        let glass = sections.iter().flat_map(|s| &s.rows).find(|r| r.label == MaterialType::Glass.label()).expect("glass");
        assert_eq!(glass.item, Item::Node(NodeKind::Shader(Box::new(MaterialType::Glass.defaults())), Some(MaterialType::Glass.label().to_string())));

        let scene: Vec<&Row> = sections.iter().filter(|s| s.group == Group::Scene).flat_map(|s| &s.rows).collect();
        assert_eq!(scene.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["Shell", "Felt Grey"]);
        assert_eq!(scene.iter().map(|r| r.current).collect::<Vec<_>>(), [false, true]);
    }

    /// The search is over everything at once, the operations inside a node
    /// with it, and leaves out what it did not find.
    #[test]
    fn the_browsers_search_finds_nodes_operations_and_materials() {
        let entries = entries();
        let materials = [(MaterialKey(1), "Shell".to_string()), (MaterialKey(2), "Felt Grey".to_string())];
        let found = sections(&entries, &materials, None, "noise");
        assert_eq!(labels(&found, Group::Maps).first().map(String::as_str), Some("Noise Texture"), "the best match leads");
        assert!(labels(&found, Group::Scene).is_empty());
        assert!(labels(&found, Group::Maps).iter().all(|l| l.to_lowercase().contains("noise")), "{:?}", labels(&found, Group::Maps));
        assert!(!sections(&entries, &materials, None, "vm").is_empty(), "initials still find Vector Math");
        assert!(found.iter().all(|s| !s.rows.is_empty()), "a heading with nothing found is left out");

        let found = sections(&entries, &materials, None, "multiply");
        assert!(labels(&found, Group::Utilities).iter().any(|l| l == "Math › Multiply"), "{:?}", labels(&found, Group::Utilities));

        let found = sections(&entries, &materials, None, "felt");
        assert_eq!(labels(&found, Group::Scene), ["Felt Grey"]);

        let found = sections(&entries, &materials, None, "glass");
        assert!(labels(&found, Group::Materials).iter().any(|l| l == MaterialType::Glass.label()));

        assert!(sections(&entries, &materials, None, "qqqzzz").is_empty());
    }
}
