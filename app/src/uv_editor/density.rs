//! The Texel density panel: how fine a texture of a given size is on the
//! selected islands, or on the faces picked on the part, in pixels per
//! centimetre of the part - its own metres, as the rulers read them - read
//! into a field and set on islands from it; and how much of the layout's
//! square the islands cover, and how much of that they overlap.

use super::layout::Layout;
use super::{Edits, Part, UvEditor};
use crate::project::Project;
use tequila_scene::uv::arrange;
use tequila_scene::uv::density::{self, Coverage};
use tequila_scene::MeshKey;

/// The texture sizes offered, as the layout export offers them.
pub const SIZES: [(u32, &str); 5] = [(512, "512"), (1024, "1K"), (2048, "2K"), (4096, "4K"), (8192, "8K")];

/// A grid of this many cells a side finds the coverage: to a tenth of a
/// per cent, in a few milliseconds.
const COVERAGE_CELLS: usize = 1024;

#[derive(Clone, Debug)]
pub struct Texel {
    pub texture: u32,
    /// The density typed or read, in pixels per centimetre.
    pub field: f32,
    /// The coverage of the layout it was worked out for.
    coverage: Option<((MeshKey, usize), Coverage)>,
}

impl Texel {
    /// The coverage last worked out, where it was.
    pub fn known_coverage(&self) -> Option<Coverage> {
        self.coverage.map(|(_, c)| c)
    }
}

impl Default for Texel {
    fn default() -> Self {
        Texel { texture: 2048, field: 10.0, coverage: None }
    }
}

impl UvEditor {
    /// The density of what the panel reads - the faces picked on the part,
    /// else the selected islands, else every island - in layout units per
    /// metre, and what it was read from.
    pub(super) fn density_of(&self, part: &Part, layout: &Layout, mesh: Option<&tequila_scene::Mesh>) -> Option<(f32, String)> {
        let plural = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
        if let (false, Some(mesh)) = (part.faces.is_empty(), mesh) {
            let faces: Vec<u32> = part.faces.iter().copied().filter(|&f| (f as usize) < layout.uv.island_of.len()).collect();
            return density::of_triangles(&layout.uv, mesh, &faces).map(|d| (d, format!("the {} picked on the part", plural(faces.len(), "face", "faces"))));
        }
        let chosen = self.chosen_islands(layout);
        if chosen.is_empty() {
            let all: Vec<u32> = (0..layout.uv.islands.len() as u32).collect();
            return arrange::linear_density(&layout.uv, &all).map(|d| (d, "every island".to_string()));
        }
        arrange::linear_density(&layout.uv, &chosen).map(|d| (d, format!("the {} selected", plural(chosen.len(), "island", "islands"))))
    }

    /// The density read into the field.
    pub(super) fn density_get(&mut self, part: &Part, layout: &Layout, mesh: &tequila_scene::Mesh) {
        match self.density_of(part, layout, Some(mesh)) {
            Some((d, what)) => {
                self.texel.field = density::pixels_per_cm(d, self.texel.texture);
                self.say(&format!("Read {:.2} px/cm at {} from {what}", self.texel.field, size_name(self.texel.texture)), false);
            }
            None => self.say("Could not read the density: what is selected has no area", true),
        }
    }

    /// The selected islands, or all, scaled about their middles to
    /// `pixels` per centimetre at the panel's texture size.
    pub(super) fn density_apply(&mut self, pixels: f32, part: &Part, layout: &Layout, edits: &mut Edits) {
        let islands = self.islands_or_all(layout);
        let target = density::from_pixels_per_cm(pixels, self.texel.texture);
        match arrange::set_density(&layout.uv, &islands, target) {
            Ok(plan) if plan.is_empty() => self.say(&format!("The islands are at {pixels:.2} px/cm already"), false),
            Ok(plan) => {
                let done = format!("Set {} to {pixels:.2} px/cm at {}", if islands.len() == 1 { "1 island".to_string() } else { format!("{} islands", islands.len()) }, size_name(self.texel.texture));
                let command = tequila_scene::Command::PlaceUvIslands { node: part.node, set: layout.uv.set, islands: plan.islands, maps: plan.maps };
                edits.layout.push(super::LayoutEdit { command, verb: "set the density", done, carry: true });
            }
            Err(error) => self.say(&format!("Could not set the density: {error}"), true),
        }
    }

    /// The layout's coverage, worked out again when its mesh or set is not
    /// the one it was for.
    pub(super) fn coverage(&mut self, part: &Part, layout: &Layout) -> Coverage {
        let key = (part.mesh, layout.uv.set);
        match self.texel.coverage {
            Some((k, c)) if k == key => c,
            _ => {
                let c = density::coverage(&layout.uv, COVERAGE_CELLS);
                self.texel.coverage = Some((key, c));
                c
            }
        }
    }

    pub(super) fn density_panel(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&Layout>, project: &Project) {
        let p = crate::theme::p();
        let Some(layout) = layout else {
            ui.weak("Unwrap the part to read its texel density.");
            return;
        };
        let mesh = project.document().geometry.get(part.mesh);
        let mut size = self.texel.texture;
        let labels: Vec<(u32, &str)> = SIZES.to_vec();
        if crate::theme::segmented(ui, &mut size, &labels) {
            // The same density, read at the new size.
            self.texel.field *= size as f32 / self.texel.texture as f32;
            self.texel.texture = size;
        }
        let now = self.density_of(part, layout, mesh);
        ui.add_space(2.0);
        match &now {
            Some((d, what)) => {
                ui.label(egui::RichText::new(format!("{:.2} px/cm", density::pixels_per_cm(*d, self.texel.texture))).font(crate::theme::strong(15.0)));
                ui.label(egui::RichText::new(format!("On {what}, at {}", size_name(self.texel.texture))).size(12.0).color(p.text_dim));
            }
            None => {
                ui.label(egui::RichText::new("No area to read").size(12.0).color(p.text_dim));
            }
        }
        ui.horizontal(|ui| {
            let get = icons_button(ui, crate::icons::Icon::Pipette, "Get", "Read the density shown into the field");
            if get {
                if let Some((d, what)) = &now {
                    self.texel.field = density::pixels_per_cm(*d, self.texel.texture);
                    let field = self.texel.field;
                    self.say(&format!("Read {field:.2} px/cm from {what}"), false);
                }
            }
            ui.add(egui::DragValue::new(&mut self.texel.field).range(0.01..=100_000.0).speed(0.05).max_decimals(2).suffix(" px/cm"))
                .on_hover_text("A density, in pixels of the texture per centimetre of the part");
            if icons_button(ui, crate::icons::Icon::Check, "Apply", "Scale the selected islands, or all, about their middles to this density") {
                self.arrange.pending.push(super::arrange_tools::Request::DensityApply(self.texel.field));
            }
        });
        let coverage = self.coverage(part, layout);
        ui.add_space(2.0);
        ui.label(egui::RichText::new(format!("Covers {:.1}% of the square", coverage.covered * 100.0)).size(12.5).color(p.text));
        let overlap = if coverage.overlap > 0.0005 { format!("{:.1}% overlaps", coverage.overlap * 100.0) } else { "Nothing overlaps".to_string() };
        ui.label(egui::RichText::new(overlap).size(12.0).color(if coverage.overlap > 0.0005 { p.danger } else { p.text_dim }))
            .on_hover_text("The share of the square more than one island covers: there the texture prints on both");
    }
}

fn icons_button(ui: &mut egui::Ui, icon: crate::icons::Icon, label: &str, detail: &str) -> bool {
    crate::icons::tip(crate::icons::labelled(ui, icon, label, false), label, detail, "").clicked()
}

/// A texture size, as the panel names it.
pub fn size_name(size: u32) -> String {
    SIZES.iter().find(|(s, _)| *s == size).map(|(_, n)| n.to_string()).unwrap_or_else(|| format!("{size} px"))
}
