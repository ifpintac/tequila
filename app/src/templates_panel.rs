//! The Material Templates section (`docs/04` §3.3): rules that assign
//! materials across a whole assembly, previewed before anything changes.
//!
//! The rules and their matching are [`tequila_scene::templates`]'s; this is
//! the editing, the preview table, and the one-step apply.

use crate::project::Project;
use eframe::egui;
use tequila_core::spectrum::Spectrum;
use tequila_scene::templates::{commands, plan, Condition, MaterialTemplate, Rule};
use tequila_scene::Command;

/// Rows shown in the preview before it says how many more there are.
const PREVIEW_ROWS: usize = 200;

#[derive(Default)]
pub struct TemplatesPanel {
    /// The template being edited and previewed.
    open: Option<usize>,
}

impl TemplatesPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        // Open from the start in a project that has templates to use.
        let any = !project.document().material_templates.is_empty();
        crate::field::Fold::new(egui::RichText::new("Material templates").strong())
            .default_open(any)
            .show(ui, |ui| self.body(ui, project));
    }

    fn body(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        if self.open.is_none() && !project.document().material_templates.is_empty() {
            self.open = Some(0);
        }
        ui.small(
            "Rules that give every matching part a material at once: by part name, \
             by the material or colour the CAD file gave it, or by a metadata field \
             such as its layer. The first rule that matches a part decides it.",
        );
        let original = project.document().material_templates.clone();
        let mut templates = original.clone();
        let materials: Vec<(tequila_scene::MaterialKey, String)> = project
            .document()
            .materials
            .iter()
            .map(|(key, definition)| (key, definition.name.clone()))
            .collect();

        ui.horizontal(|ui| {
            if ui.button("+ Template").clicked() {
                templates.push(MaterialTemplate {
                    name: format!("Template {}", templates.len() + 1),
                    rules: Vec::new(),
                });
                self.open = Some(templates.len() - 1);
            }
            let names: Vec<String> = templates.iter().map(|t| t.name.clone()).collect();
            if !names.is_empty() {
                let current = self.open.filter(|&i| i < names.len());
                egui::ComboBox::from_id_salt("template choice")
                    .selected_text(current.map_or("Choose…".to_string(), |i| names[i].clone()))
                    .show_ui(ui, |ui| {
                        for (index, name) in names.iter().enumerate() {
                            if ui.selectable_label(current == Some(index), name).clicked() {
                                self.open = Some(index);
                            }
                        }
                    });
            }
        });

        let mut apply = None;
        if let Some(index) = self.open.filter(|&i| i < templates.len()) {
            let mut delete = false;
            let template = &mut templates[index];
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut template.name).desired_width(140.0));
                if ui.small_button("×").on_hover_text("Delete this template").clicked() {
                    delete = true;
                }
            });
            rules(ui, template, &materials);

            // ---- The preview ------------------------------------------------
            let planned = plan(project.document(), template);
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("Would change {} parts", planned.len())).strong());
            if !planned.is_empty() {
                let name_of = |key: Option<tequila_scene::MaterialKey>| {
                    key.and_then(|k| materials.iter().find(|(m, _)| *m == k))
                        .map_or("(none)".to_string(), |(_, n)| n.clone())
                };
                egui::ScrollArea::vertical().max_height(220.0).id_salt("template preview").show(ui, |ui| {
                    egui::Grid::new("template preview grid").striped(true).num_columns(3).show(ui, |ui| {
                        ui.weak("Part");
                        ui.weak("Now");
                        ui.weak("Becomes");
                        ui.end_row();
                        for line in planned.iter().take(PREVIEW_ROWS) {
                            let part = project.document().tree.get(line.part).map_or(String::new(), |n| n.name.clone());
                            ui.label(part);
                            ui.label(name_of(line.from));
                            ui.label(format!("{} (rule {})", name_of(Some(line.to)), line.rule + 1));
                            ui.end_row();
                        }
                    });
                    if planned.len() > PREVIEW_ROWS {
                        ui.weak(format!("… and {} more", planned.len() - PREVIEW_ROWS));
                    }
                });
                if ui
                    .button(format!("Apply to {} parts", planned.len()))
                    .on_hover_text("One undo step.")
                    .clicked()
                {
                    apply = Some(planned);
                }
            }
            if delete {
                templates.remove(index);
                self.open = None;
            }
        }

        if templates != original {
            if let Err(error) = project.apply(Command::SetMaterialTemplates { templates }) {
                project.status = error.to_string();
            }
        }
        if let Some(planned) = apply {
            let count = planned.len();
            project.begin_group();
            let mut result = Ok(());
            for command in commands(&planned) {
                result = project.apply(command);
                if result.is_err() {
                    break;
                }
            }
            project.end_group();
            project.status = match result {
                Ok(()) => format!("Assigned materials to {count} parts"),
                Err(error) => error.to_string(),
            };
        }
    }
}

/// A template's rules, each a condition and a material.
fn rules(ui: &mut egui::Ui, template: &mut MaterialTemplate, materials: &[(tequila_scene::MaterialKey, String)]) {
    let mut remove = None;
    let mut swap = None;
    let count = template.rules.len();
    for (index, rule) in template.rules.iter_mut().enumerate() {
        ui.push_id(("rule", index), |ui| {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(format!("{}.", index + 1));
                egui::ComboBox::from_id_salt("condition kind")
                    .selected_text(rule.condition.label())
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        let choices = [
                            Condition::Name("*".into()),
                            Condition::Material("*".into()),
                            Condition::Colour {
                                color: Spectrum::splat(0.5),
                                tolerance: 0.02,
                            },
                            Condition::Metadata {
                                key: "layer".into(),
                                value: "*".into(),
                            },
                        ];
                        for choice in choices {
                            let same = std::mem::discriminant(&choice) == std::mem::discriminant(&rule.condition);
                            if ui.selectable_label(same, choice.label()).clicked() && !same {
                                rule.condition = choice;
                            }
                        }
                    });
                if ui.small_button("Up").clicked() && index > 0 {
                    swap = Some((index, index - 1));
                }
                if ui.small_button("Down").clicked() && index + 1 < count {
                    swap = Some((index, index + 1));
                }
                if ui.small_button("×").on_hover_text("Remove this rule").clicked() {
                    remove = Some(index);
                }
            });
            ui.horizontal(|ui| {
                match &mut rule.condition {
                    Condition::Name(pattern) | Condition::Material(pattern) => {
                        ui.add(egui::TextEdit::singleline(pattern).hint_text("bolt*").desired_width(140.0))
                            .on_hover_text("* matches any run of characters, ? any one. Not case-sensitive.");
                    }
                    Condition::Colour { color, tolerance } => {
                        crate::color_picker::edit(ui, color);
                        ui.add(crate::field::Slider::new(tolerance, 0.0..=0.25).text("within"));
                    }
                    Condition::Metadata { key, value } => {
                        ui.add(egui::TextEdit::singleline(key).hint_text("layer").desired_width(70.0));
                        ui.label("=");
                        ui.add(egui::TextEdit::singleline(value).hint_text("HARDWARE").desired_width(80.0));
                    }
                }
                ui.label("gets");
                let current = materials.iter().find(|(k, _)| *k == rule.material).map_or("(missing)".to_string(), |(_, n)| n.clone());
                egui::ComboBox::from_id_salt("rule material")
                    .selected_text(current)
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for (key, name) in materials {
                            ui.selectable_value(&mut rule.material, *key, name);
                        }
                    });
            });
        });
    }
    if let Some(index) = remove {
        template.rules.remove(index);
    }
    if let Some((a, b)) = swap {
        template.rules.swap(a, b);
    }
    if ui
        .add_enabled(!materials.is_empty(), egui::Button::new("+ Rule"))
        .on_disabled_hover_text("The project has no materials to assign yet.")
        .clicked()
    {
        template.rules.push(Rule {
            condition: Condition::Name("*".into()),
            material: materials[0].0,
        });
    }
}
