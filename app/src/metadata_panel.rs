//! The Scene tab's metadata panel (`docs/03` §6.5): a part's CAD attributes
//! and notes, editable; every part with the same value, a click away; and the
//! tree as a spreadsheet.
//!
//! With several parts selected the panel shows every field any of them has.
//! A value they share is shown; one they disagree on reads "Mixed". Editing
//! a field sets it on all of them, as one undo step - the way a part number
//! or a supplier gets written onto a whole subassembly at once.

use crate::project::Project;
use crate::theme;
use eframe::egui;
use tequila_scene::metadata;
use tequila_scene::tree::NodeId;
use tequila_scene::Command;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default)]
pub struct MetadataPanel {
    /// The field being typed into: its key, the text so far, and the parts
    /// it will be written to - held from when typing began, so a click on
    /// another row, which ends the typing, does not move the edit there.
    editing: Option<Edit>,
    new_key: String,
    new_value: String,
    /// Columns left out of the CSV export.
    csv_hidden: BTreeSet<String>,
    /// Every field and how many parts have it, with the document and the
    /// history change it was counted at: counted every frame, it walked
    /// every node of an assembly to draw a panel that had not changed.
    keys: Option<((u64, u64), BTreeMap<String, usize>)>,
}

#[derive(Debug)]
struct Edit {
    key: String,
    text: String,
    nodes: Vec<NodeId>,
}

/// A field across the selection: its one value, or `None` when the parts
/// disagree or some lack it.
fn fields(project: &Project, nodes: &[NodeId]) -> BTreeMap<String, Option<String>> {
    let mut out: BTreeMap<String, Option<String>> = BTreeMap::new();
    let tree = &project.document().tree;
    let all: Vec<_> = nodes.iter().filter_map(|&id| tree.get(id)).collect();
    for node in &all {
        for key in node.metadata.keys() {
            out.entry(key.clone()).or_insert_with(|| {
                let first = node.metadata.get(key);
                all.iter().all(|n| n.metadata.get(key) == first).then(|| first.cloned()).flatten()
            });
        }
    }
    out
}

/// Set (or with `None`, remove) `key` on every node, as one undo step.
fn write(project: &mut Project, nodes: &[NodeId], key: &str, value: Option<&str>) {
    let commands: Vec<Command> = nodes
        .iter()
        .filter(|&&id| {
            let current = project.document().tree.get(id).and_then(|n| n.metadata.get(key));
            current.map(String::as_str) != value
        })
        .map(|&node| Command::SetMetadata { node, key: key.to_string(), value: value.map(str::to_string) })
        .collect();
    if commands.is_empty() {
        return;
    }
    let grouped = commands.len() > 1;
    if grouped {
        project.begin_group();
    }
    for command in commands {
        if let Err(error) = project.apply(command) {
            project.status = error.to_string();
        }
    }
    if grouped {
        project.end_group();
    }
}

/// Every node whose `key` is exactly `value`, regardless of case.
fn same(project: &Project, key: &str, value: &str) -> Vec<NodeId> {
    let tree = &project.document().tree;
    tree.depth_first()
        .into_iter()
        .filter(|&id| {
            tree.get(id).is_some_and(|n| {
                n.metadata.iter().any(|(k, v)| k.trim().eq_ignore_ascii_case(key.trim()) && v.trim().eq_ignore_ascii_case(value.trim()))
            })
        })
        .collect()
}

impl MetadataPanel {
    /// `search` is the Scene tab's search box, which a field name can be
    /// sent to.
    pub fn show(&mut self, ui: &mut egui::Ui, project: &mut Project, search: &mut String) {
        let selected: Vec<NodeId> = project.selection.nodes().collect();
        let counted_at = (project.document_epoch(), project.bus().changes());
        let keys = match &self.keys {
            Some((at, keys)) if *at == counted_at => keys.clone(),
            _ => {
                let keys = tequila_scene::command::metadata_keys(project.document());
                self.keys = Some((counted_at, keys.clone()));
                keys
            }
        };

        ui.horizontal(|ui| {
            let what = match selected.len() {
                0 => "Nothing selected".to_string(),
                1 => project.document().tree.get(selected[0]).map(|n| n.name.clone()).unwrap_or_default(),
                n => format!("{n} selected"),
            };
            ui.label(egui::RichText::new(what).color(theme::p().text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.export_menu(ui, project, &keys);
            });
        });

        if selected.is_empty() {
            self.overview(ui, &keys, search);
            return;
        }

        let fields = fields(project, &selected);
        if self.editing.as_ref().is_some_and(|e| !fields.contains_key(&e.key)) {
            self.editing = None;
        }
        let mut remove: Option<String> = None;
        let mut commit: Option<Edit> = None;
        let mut select: Option<(String, String)> = None;
        if fields.is_empty() {
            ui.weak(if selected.len() == 1 { "No fields yet. A CAD file's attributes land here; add your own below." } else { "None of these has a field yet." });
        }
        // Rows of key, value, and the row's two actions, measured out by
        // hand: a grid gives a text field no width to fill.
        let width = ui.available_width();
        let gap = ui.spacing().item_spacing.x;
        let key_width = (width * 0.34).clamp(64.0, 150.0);
        let actions = 20.0 + gap + 20.0;
        let value_width = (width - key_width - actions - 3.0 * gap).max(40.0);
        let height = 24.0;
        for (key, value) in &fields {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(egui::vec2(key_width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_min_width(key_width);
                    ui.add(egui::Label::new(egui::RichText::new(key).color(theme::p().text_dim)).truncate())
                        .on_hover_text(key.as_str());
                });
                let editing_this = self.editing.as_ref().is_some_and(|e| &e.key == key);
                let mut scratch = value.clone().unwrap_or_default();
                let text = match &mut self.editing {
                    Some(edit) if editing_this => &mut edit.text,
                    _ => &mut scratch,
                };
                let mut editor = egui::TextEdit::singleline(text);
                if value.is_none() {
                    editor = editor.hint_text(egui::RichText::new("Mixed").italics().color(theme::p().text_faint));
                }
                let response = ui.add_sized([value_width, height], editor);
                if response.has_focus() && !editing_this {
                    self.editing = Some(Edit { key: key.clone(), text: scratch.clone(), nodes: selected.clone() });
                }
                if response.lost_focus() && editing_this {
                    if let Some(edit) = self.editing.take() {
                        // Escape gives up; anything else that ends the
                        // typing keeps it.
                        let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        let unchanged = value.as_deref() == Some(edit.text.as_str()) || (value.is_none() && edit.text.is_empty());
                        if !cancelled && !unchanged {
                            commit = Some(edit);
                        }
                    }
                }
                let can_select = value.as_ref().is_some_and(|v| !v.trim().is_empty());
                let find = ui
                    .add_enabled_ui(can_select, |ui| crate::icons::button(ui, crate::icons::Icon::Search, false, 20.0, egui::Sense::click()))
                    .inner;
                let find = match value {
                    Some(v) if can_select => find.on_hover_text(format!("Select every part whose {key} is {v}")),
                    _ => find.on_disabled_hover_text("The selection disagrees on this field"),
                };
                if find.clicked() {
                    select = value.clone().map(|v| (key.clone(), v));
                }
                if crate::icons::button(ui, crate::icons::Icon::Close, false, 20.0, egui::Sense::click())
                    .on_hover_text(if selected.len() > 1 { "Remove this field from all of them" } else { "Remove this field" })
                    .clicked()
                {
                    remove = Some(key.clone());
                }
            });
        }

        // A new field.
        ui.add_space(2.0);
        let mut add = false;
        ui.horizontal(|ui| {
            let key = ui.add_sized([key_width, height], egui::TextEdit::singleline(&mut self.new_key).hint_text("New field"));
            let value = ui.add_sized([value_width, height], egui::TextEdit::singleline(&mut self.new_value).hint_text("Value"));
            let entered = (key.lost_focus() || value.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ready = !self.new_key.trim().is_empty();
            let clicked = ui
                .add_sized([actions, height], egui::Button::new("Add"))
                .on_hover_text(if selected.len() > 1 { "Add to every selected part" } else { "Add to this part" })
                .clicked();
            add = ready && (clicked || entered);
        });

        if let Some(edit) = commit {
            write(project, &edit.nodes, &edit.key, Some(edit.text.trim()));
        }
        if let Some(key) = remove {
            self.editing = None;
            write(project, &selected, &key, None);
        }
        if add {
            let key = self.new_key.trim().to_string();
            let value = self.new_value.trim().to_string();
            write(project, &selected, &key, Some(&value));
            self.new_key.clear();
            self.new_value.clear();
        }
        if let Some((key, value)) = select {
            let nodes = same(project, &key, &value);
            project.status = format!("Selected {} with {key} = {value}", plural(nodes.len()));
            project.selection.select_many(nodes, tequila_scene::selection::SelectionMode::Replace);
        }
    }

    /// With nothing selected: the fields the scene has, each a search away.
    fn overview(&mut self, ui: &mut egui::Ui, keys: &BTreeMap<String, usize>, search: &mut String) {
        if keys.is_empty() {
            ui.weak("No part has metadata yet. CAD attributes arrive with the model; select a part to add your own.");
            return;
        }
        ui.weak("Click a field to find the parts that have it, or search Field = value.");
        ui.horizontal_wrapped(|ui| {
            for (key, count) in keys {
                if ui
                    .add(egui::Button::new(format!("{key} · {count}")).small())
                    .on_hover_text(format!("Show the {} with a {key}", plural(*count)))
                    .clicked()
                {
                    *search = format!("{key} =");
                }
            }
        });
    }

    /// The CSV export, with a choice of which fields become columns.
    fn export_menu(&mut self, ui: &mut egui::Ui, project: &mut Project, keys: &BTreeMap<String, usize>) {
        ui.menu_button("Export CSV", |ui| {
            ui.set_min_width(200.0);
            if keys.is_empty() {
                ui.weak("No fields: the export will hold the tree alone.");
            } else {
                ui.label(egui::RichText::new("Columns").color(theme::p().text_dim));
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    for key in keys.keys() {
                        let mut on = !self.csv_hidden.contains(key);
                        if ui.checkbox(&mut on, key).changed() {
                            if on {
                                self.csv_hidden.remove(key);
                            } else {
                                self.csv_hidden.insert(key.clone());
                            }
                        }
                    }
                });
            }
            ui.separator();
            if ui.button("Save CSV…").clicked() {
                ui.close_menu();
                let columns: Vec<String> = keys.keys().filter(|k| !self.csv_hidden.contains(*k)).cloned().collect();
                let name = project
                    .path
                    .as_ref()
                    .and_then(|p| p.file_stem())
                    .map(|s| format!("{} parts.csv", s.to_string_lossy()))
                    .unwrap_or_else(|| "Scene parts.csv".into());
                if let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).set_file_name(&name).save_file() {
                    let text = metadata::csv(project.document(), &columns);
                    let rows = text.matches("\r\n").count().saturating_sub(1);
                    project.status = match std::fs::write(&path, text) {
                        Ok(()) => format!("Exported {rows} rows to {}", path.display()),
                        Err(error) => format!("Could not export the CSV: {error}"),
                    };
                }
            }
        })
        .response
        .on_hover_text("The scene tree as a spreadsheet, with the fields you choose as columns");
    }
}

fn plural(n: usize) -> String {
    if n == 1 { "1 part".into() } else { format!("{n} parts") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scene::tree::NodeKind;

    fn project() -> (Project, NodeId, NodeId) {
        let project = Project::with_document(
            tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2),
            std::sync::Arc::new(tequila_scene::demo::studio_environment()),
        );
        let tree = &project.document().tree;
        let group = tree.iter().find(|n| n.kind == NodeKind::Group).map(|n| n.id).expect("a group");
        let part = tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| n.id).expect("a part");
        (project, group, part)
    }

    /// Across a selection a shared value shows, a disagreement does not, and
    /// a field only some of them have counts as a disagreement.
    #[test]
    fn a_selection_shows_what_it_shares() {
        let (mut project, group, part) = project();
        write(&mut project, &[part], "Material", Some("ABS"));
        write(&mut project, &[group], "Material", Some("PC"));
        write(&mut project, &[group, part], "Supplier", Some("Acme"));
        write(&mut project, &[part], "Finish", Some("Matte"));
        let fields = fields(&project, &[group, part]);
        assert_eq!(fields.get("Material"), Some(&None));
        assert_eq!(fields.get("Supplier"), Some(&Some("Acme".to_string())));
        assert_eq!(fields.get("Finish"), Some(&None), "only one of them has it");
        assert_eq!(same(&project, "supplier", "ACME"), vec![group, part]);
    }

    /// Writing a field onto several parts is one step to undo, and removing
    /// it from them another.
    #[test]
    fn a_field_written_to_many_is_one_undo_step() {
        let (mut project, group, part) = project();
        let has = |p: &Project, id: NodeId| p.document().tree.get(id).unwrap().metadata.get("Supplier").cloned();
        write(&mut project, &[group, part], "Supplier", Some("Acme"));
        assert_eq!((has(&project, group), has(&project, part)), (Some("Acme".into()), Some("Acme".into())));
        write(&mut project, &[group, part], "Supplier", None);
        assert_eq!((has(&project, group), has(&project, part)), (None, None));
        project.undo();
        assert_eq!((has(&project, group), has(&project, part)), (Some("Acme".into()), Some("Acme".into())), "the removal came back whole");
        project.undo();
        assert_eq!((has(&project, group), has(&project, part)), (None, None), "and the write went whole");
    }
}
