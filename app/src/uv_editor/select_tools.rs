//! The toolbar's Select menu: islands of the same shape as the selected,
//! the border edges of islands, the edges square to the layout, the faces
//! with no area on it, and the faces turned over against their island.
//! What it finds the engine works out (`tequila_scene::uv::select`); islands
//! and edges are selected here, faces are picked on the part, as the
//! viewport picks them at Face level, and drawn on both.

use super::arrange_tools::Request;
use super::layout::Layout;
use super::selection::{Item, Mode};
use super::{Edits, Part, UvEditor};
use crate::icons::{self, Icon};
use tequila_scene::uv::select::{self, Square};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Similar,
    Borders,
    Across,
    Down,
    NoArea,
    Flipped,
}

impl Pick {
    pub const ALL: [Pick; 6] = [Pick::Similar, Pick::Borders, Pick::Across, Pick::Down, Pick::NoArea, Pick::Flipped];

    pub fn name(self) -> &'static str {
        match self {
            Pick::Similar => "Similar islands",
            Pick::Borders => "Island borders",
            Pick::Across => "Horizontal edges",
            Pick::Down => "Vertical edges",
            Pick::NoArea => "Faces with no area",
            Pick::Flipped => "Flipped faces",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Pick::Similar => "The islands of the same piece and shape as the selected ones, within the shape tolerance: a part's repeated or mirrored pieces",
            Pick::Borders => "The edges round the selected islands, or round all of them: their seams and the part's own edge",
            Pick::Across => "Of the selected edges, or those of the selected islands or all, the ones running across the layout within the edge tolerance",
            Pick::Down => "Of the selected edges, or those of the selected islands or all, the ones running down the layout within the edge tolerance",
            Pick::NoArea => "Faces squashed to nothing on the layout: artwork cannot land on them. Picked on the part",
            Pick::Flipped => "Faces turned over against the rest of their island, where artwork prints mirrored. Picked on the part",
        }
    }
}

impl UvEditor {
    /// The Select menu, in the toolbar.
    pub(super) fn select_menu(&mut self, ui: &mut egui::Ui, layout: Option<&Layout>) {
        let id = ui.id().with("uv-select-menu");
        let response = ui.add_enabled_ui(layout.is_some(), |ui| icons::labelled(ui, Icon::SelectMenu, "Select", false)).inner;
        let response = icons::tip(response, "Select", "Select islands, edges or faces by what they are like", "");
        if response.clicked() {
            ui.memory_mut(|m| m.toggle_popup(id));
        }
        // A script's asking, once.
        match self.select_open.take() {
            Some(true) => ui.memory_mut(|m| m.open_popup(id)),
            Some(false) if ui.memory(|m| m.is_popup_open(id)) => ui.memory_mut(|m| m.close_popup()),
            _ => {}
        }
        egui::popup::popup_below_widget(ui, id, &response, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_min_width(230.0);
            for pick in Pick::ALL {
                if pick == Pick::NoArea || pick == Pick::Across {
                    ui.separator();
                }
                let button = ui.add(egui::Button::new(pick.name()).frame(false)).on_hover_text(pick.detail());
                if button.clicked() {
                    self.arrange.pending.push(Request::Select(pick));
                }
            }
        });
    }

    /// Select what `pick` finds.
    pub(super) fn select_pick(&mut self, pick: Pick, part: &Part, layout: &Layout, mesh: &tequila_scene::Mesh, edits: &mut Edits) {
        let plural = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
        let uv = &layout.uv;
        let islands_or_all = self.islands_or_all(layout);
        let edge_items = |edges: &[u32]| edges.iter().map(|&e| Item::Edge(uv.edges[e as usize].vertices)).collect::<Vec<_>>();
        match pick {
            Pick::Similar => {
                let chosen = self.chosen_islands(layout);
                if chosen.is_empty() {
                    self.say("Could not select similar islands: select an island to find the others like it", true);
                    return;
                }
                let found = select::similar_islands(uv, mesh, &chosen, self.arrange.similar, self.arrange.mirrored);
                self.set_mode(Mode::Islands);
                self.selection.set_all(found.iter().filter_map(|&i| super::editing::island_item(layout, i)));
                let more = found.len() - chosen.len();
                self.say(&if more == 0 { format!("No other island is like the {} selected", plural(chosen.len(), "island", "islands")) } else { format!("Selected {} like the {} selected", plural(more, "more island", "more islands"), plural(chosen.len(), "island", "islands")) }, false);
            }
            Pick::Borders => {
                let edges = select::border_edges(uv, &islands_or_all);
                self.set_mode(Mode::Edges);
                self.selection.set_all(edge_items(&edges));
                self.say(&format!("Selected the {} round {}", plural(edges.len(), "border edge", "border edges"), plural(islands_or_all.len(), "island", "islands")), false);
            }
            Pick::Across | Pick::Down => {
                let picked: Vec<u32> = self.selection.edges().into_iter().filter_map(|e| layout.edge_named(e)).collect();
                let candidates: Vec<u32> = if self.mode == Mode::Edges && !picked.is_empty() {
                    picked
                } else {
                    islands_or_all.iter().flat_map(|&i| uv.islands[i as usize].edges.clone()).map(|e| e as u32).collect()
                };
                let way = if pick == Pick::Across { Square::Across } else { Square::Down };
                let found = select::square_edges(uv, &candidates, way, self.arrange.tolerance.to_radians());
                self.set_mode(Mode::Edges);
                self.selection.set_all(edge_items(&found));
                let what = if pick == Pick::Across { "running across" } else { "running down" };
                self.say(&format!("Selected {} {what} within {:.0}°", plural(found.len(), "edge", "edges"), self.arrange.tolerance), false);
            }
            Pick::NoArea | Pick::Flipped => {
                let faces = if pick == Pick::NoArea { select::no_area_faces(uv, mesh) } else { select::flipped_faces(uv) };
                if faces.is_empty() {
                    self.say(if pick == Pick::NoArea { "No face has no area on the layout" } else { "No face is turned over against its island" }, false);
                    return;
                }
                let what = if pick == Pick::NoArea { "with no area" } else { "turned over" };
                self.say(&format!("Picked {} {what} on the part", plural(faces.len(), "face", "faces")), false);
                edits.faces = Some(faces);
            }
        }
        let _ = part;
    }
}
