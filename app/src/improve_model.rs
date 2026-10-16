//! The Improve model window: more resolution for a model that came in
//! coarse (`tequila_scene::refine`).
//!
//! Three tools, each one step to undo:
//! - **Smoother curves**, for a CAD model whose exact surfaces are kept:
//!   divide them again, finer. The exact way, and the one to try first on a
//!   STEP that came in faceted.
//! - **Subdivide**, for any mesh: every triangle becomes four, the new
//!   points following the curve the shading describes.
//! - **Shade again**, for an STL or an OBJ that looks faceted because its
//!   file had no normals, or flat ones.

use eframe::egui;
use tequila_import::step::Tessellation;
use tequila_scene::refine;
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::{Command, SceneDocument};

/// What the window asks the application to do.
pub enum Improve {
    /// Divide the CAD model the node belongs to again, at this quality: the
    /// application's own re-tessellation, read on another thread.
    Retessellate(NodeId, Tessellation),
    /// A change to the document, with what to say once it is made.
    Apply(Command, String),
}

pub struct ImproveModel {
    node: NodeId,
    roots: Vec<NodeId>,
    name: String,
    /// The quality its CAD surfaces were divided at, when they are kept.
    cad: Option<Tessellation>,
    levels: u32,
    crease: f32,
}

impl ImproveModel {
    /// For the model `node` belongs to; `None` when it has no parts.
    pub fn new(document: &SceneDocument, node: NodeId) -> Option<ImproveModel> {
        let roots = tequila_scene::reimport::import_roots(document, node);
        if refine::parts_under(document, &roots).is_empty() {
            return None;
        }
        let top = roots.first().and_then(|&root| document.tree.get(root));
        let name = top.map(|n| n.name.clone()).unwrap_or_else(|| "The model".into());
        let cad = top
            .and_then(|n| n.metadata.get(tequila_import::step::TESSELLATION))
            .and_then(|label| Tessellation::from_label(label));
        Some(ImproveModel { node, roots, name, cad, levels: 1, crease: 30.0 })
    }

    /// The window. Returns what to do, if anything, and whether it is still
    /// open.
    pub fn show(&mut self, ctx: &egui::Context, document: &SceneDocument) -> (Option<Improve>, bool) {
        let parts = refine::parts_under(document, &self.roots);
        let triangles: usize = parts.iter().map(|&id| triangles_of(document, id)).sum();
        let (mut open, mut done, mut action) = (true, false, None);
        egui::Window::new("Improve model")
            .id(egui::Id::new("improve-model"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.label(format!("{}: {} parts, {} triangles.", self.name, parts.len(), thousands(triangles)));
                let note = |ui: &mut egui::Ui, text: &str| {
                    ui.label(egui::RichText::new(text).small().color(crate::theme::dim()));
                };

                if let Some(current) = self.cad {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Smoother curves").strong());
                    note(ui, "Divides the exact CAD surfaces again, finer: the best fix for a STEP that came in faceted.");
                    ui.horizontal(|ui| {
                        for quality in Tessellation::ALL {
                            let chosen = quality == current;
                            if ui
                                .add_enabled(!chosen, egui::SelectableLabel::new(chosen, quality.label()))
                                .on_hover_text(match quality {
                                    Tessellation::Draft => "Fewest triangles: a big assembly seen whole",
                                    Tessellation::Medium => "How a STEP comes in",
                                    Tessellation::High => "Smooth at a close look",
                                    Tessellation::Ultra => "Smooth in extreme close-ups; many triangles",
                                })
                                .clicked()
                            {
                                action = Some(Improve::Retessellate(self.node, quality));
                            }
                        }
                    });
                }

                ui.add_space(8.0);
                ui.label(egui::RichText::new("Subdivide").strong());
                note(
                    ui,
                    "Every triangle becomes four, the new points following the curve of the surface. Flat faces \
                     and sharp edges stay as they are, and nothing already there moves.",
                );
                ui.horizontal(|ui| {
                    for levels in 1..=3u32 {
                        let after = refine::subdivided_count(triangles, levels);
                        let allowed = after <= refine::MOST_TRIANGLES;
                        let label = format!("{}x: {}", 4u32.pow(levels), short(after));
                        ui.add_enabled_ui(allowed, |ui| {
                            ui.radio_value(&mut self.levels, levels, label)
                                .on_disabled_hover_text("Too many triangles for the viewport to turn");
                        });
                    }
                });
                let after = refine::subdivided_count(triangles, self.levels);
                if ui.add_enabled(after <= refine::MOST_TRIANGLES, egui::Button::new("Subdivide")).clicked() {
                    let (levels, crease) = (self.levels, self.crease);
                    if let Some(command) = refine::replace_meshes(document, &parts, |mesh| refine::subdivide(mesh, levels, crease)) {
                        action = Some(Improve::Apply(
                            command,
                            format!("Subdivided {}: {} triangles, from {}", self.name, thousands(after), thousands(triangles)),
                        ));
                    }
                }

                ui.add_space(8.0);
                ui.label(egui::RichText::new("Smooth shading").strong());
                note(
                    ui,
                    if self.cad.is_some() {
                        "For a mesh that looks faceted. A CAD model's own shading is better: use Smoother curves instead."
                    } else {
                        "For an STL or OBJ that looks faceted: works out the shading again, smooth across gentle edges."
                    },
                );
                ui.horizontal(|ui| {
                    ui.label("Keep edges sharper than");
                    ui.add(egui::Slider::new(&mut self.crease, 5.0..=90.0).suffix("°").fixed_decimals(0));
                });
                if ui.button("Shade again").clicked() {
                    let crease = self.crease;
                    if let Some(command) = refine::replace_meshes(document, &parts, |mesh| refine::reshade(mesh, crease)) {
                        action = Some(Improve::Apply(command, format!("Shaded {} again, sharp above {crease:.0}°", self.name)));
                    }
                }

                ui.add_space(6.0);
                note(ui, "Each change is one step you can undo.");
                if ui.button("Done").clicked() {
                    done = true;
                }
            });
        // A new quality is a new reading of the file: the window keeps up.
        if let Some(Improve::Retessellate(_, quality)) = &action {
            self.cad = Some(*quality);
        }
        (action, open && !done)
    }
}

fn triangles_of(document: &SceneDocument, id: NodeId) -> usize {
    match document.tree.get(id).map(|n| &n.kind) {
        Some(NodeKind::Part { mesh }) => document.geometry.get(*mesh).map_or(0, |m| m.triangle_count()),
        _ => 0,
    }
}

/// 1234567 as "1,234,567".
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 1234567 as "1.2M", 45600 as "46k".
fn short(n: usize) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.0}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_as_people_say_them() {
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(999), "999");
        assert_eq!(short(1_234_567), "1.2M");
        assert_eq!(short(45_600), "46k");
    }

    /// The window lays out frame after frame over the demo, and does
    /// nothing without a click.
    #[test]
    fn the_window_lays_out_and_waits() {
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let part = refine::parts_under(&document, &document.tree.roots().to_vec())[0];
        let mut window = ImproveModel::new(&document, part).expect("the demo has parts");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                let (action, open) = window.show(ctx, &document);
                assert!(action.is_none() && open);
            });
        }
        assert!(ctx.memory(|m| m.area_rect(egui::Id::new("improve-model"))).is_some(), "the window is not on screen");
    }
}
