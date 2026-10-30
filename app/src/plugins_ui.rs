//! The Plugins window, the plugins' own panels, and the Plugins menu.
//!
//! The window is where a plugin is found, turned on, watched and removed;
//! the menu is where an enabled plugin's commands live; a panel is a small
//! window of fields a plugin declared, drawn here so every plugin's look
//! and behaviour is Tequila's own.

use crate::plugins::{FieldKind, Origin, Plugins};
use eframe::egui;
use tequila_format::json::Value;

/// What the person asked for in the Plugins window or menu.
#[derive(Clone, Debug)]
pub enum PluginsAction {
    Install,
    New(String),
    OpenFolder,
    Reload,
    Enable(String),
    Disable(String),
    Restart(String),
    Remove(String),
    ShowFolder(String),
    /// Run a command: from the menu, or a panel's button with its values.
    Run { plugin: String, command: String, values: Vec<(String, Value)> },
}

#[derive(Default)]
pub struct PluginsWindow {
    pub open: bool,
    new_name: String,
    /// Asking before a plugin with code is turned on.
    pub(crate) confirm_enable: Option<String>,
    /// Asking before a plugin's folder is removed.
    confirm_remove: Option<String>,
}

impl PluginsWindow {
    /// Ask the person to turn `id` on, in the window's own confirmation: what
    /// a script or an AI app asking to turn a plugin on gets. Nothing runs
    /// until they say yes.
    pub fn ask_to_enable(&mut self, id: &str) {
        self.open = true;
        self.confirm_enable = Some(id.to_string());
    }

    pub fn show(&mut self, ctx: &egui::Context, plugins: &Plugins) -> Vec<PluginsAction> {
        let mut actions = Vec::new();
        if !self.open {
            return actions;
        }
        let mut open = self.open;
        egui::Window::new("Plugins")
            .open(&mut open)
            .collapsible(false)
            .default_width(460.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                match crate::plugins::python() {
                    Some(python) => hint(ui, &format!("Plugins with code run with Python ({}). Material packs need nothing.", python.join(" "))),
                    None => {
                        ui.colored_label(crate::theme::p().busy, "Python 3 was not found: plugins with code cannot run until it is installed (python.org), or TEQUILA_PYTHON names it. Material packs still work.");
                    }
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if crate::icons::labelled(ui, crate::icons::Icon::Upload, "Install from a ZIP…", false).clicked() {
                        actions.push(PluginsAction::Install);
                    }
                    if crate::icons::labelled(ui, crate::icons::Icon::FolderOpen, "Open the folder", false).clicked() {
                        actions.push(PluginsAction::OpenFolder);
                    }
                    if crate::icons::labelled(ui, crate::icons::Icon::Refresh, "Read again", false).clicked() {
                        actions.push(PluginsAction::Reload);
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("A name for a new plugin").desired_width(220.0));
                    let ready = !self.new_name.trim().is_empty();
                    if ui
                        .add_enabled(ready, egui::Button::new("Make it from the template"))
                        .on_hover_text("A working plugin to start from: a menu command, a panel and an event, in Python")
                        .clicked()
                    {
                        actions.push(PluginsAction::New(self.new_name.trim().to_string()));
                        self.new_name.clear();
                    }
                });
                hint(
                    ui,
                    "Or have an AI write it: ask the Assistant, or an AI app once connected (Assistant, Connect an AI…), for what it should do - \"a panel that renames the selected parts with a prefix\". It writes the plugin here, and you turn it on.",
                );
                if !plugins.problems.is_empty() {
                    ui.colored_label(crate::theme::p().busy, format!("{} plugin(s) could not be read", plugins.problems.len()))
                        .on_hover_text(plugins.problems.join("\n"));
                }
                ui.separator();
                if plugins.plugins.is_empty() {
                    crate::theme::empty_state(
                        ui,
                        crate::icons::Icon::Puzzle,
                        crate::theme::hue(crate::theme::Hue::Violet),
                        "No plugins yet",
                        "Install one from a ZIP, or make one from the template - docs/plugins.md says how they work.",
                    );
                    return;
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for plugin in &plugins.plugins {
                        let id = plugin.manifest.id.clone();
                        let enabled = plugins.is_enabled(&id);
                        crate::theme::card(ui, |ui| {
                            ui.horizontal(|ui| {
                                let mut on = enabled;
                                if ui.add(crate::field::Checkbox::new(&mut on, "")).changed() {
                                    if on && plugin.runs_code() {
                                        self.confirm_enable = Some(id.clone());
                                    } else if on {
                                        actions.push(PluginsAction::Enable(id.clone()));
                                    } else {
                                        actions.push(PluginsAction::Disable(id.clone()));
                                    }
                                }
                                ui.label(egui::RichText::new(&plugin.manifest.name).strong());
                                ui.label(egui::RichText::new(format!("{} {}", plugin.manifest.version, if plugin.origin == Origin::Bundled { "· example" } else { "" })).small().color(crate::theme::dim()));
                            });
                            if !plugin.manifest.description.is_empty() {
                                hint(ui, &plugin.manifest.description);
                            }
                            let mut what = Vec::new();
                            if plugin.runs_code() {
                                what.push("Python");
                            }
                            if plugin.manifest.materials.is_some() {
                                what.push("materials");
                            }
                            if !plugin.manifest.panels.is_empty() {
                                what.push("panels");
                            }
                            let state = if !enabled {
                                "off".to_string()
                            } else if let Some(reason) = plugins.failures.get(&id) {
                                format!("not running: {reason}")
                            } else if plugins.hosts.contains_key(&id) {
                                "running".to_string()
                            } else if plugin.runs_code() {
                                "starting".to_string()
                            } else {
                                "on".to_string()
                            };
                            let author = if plugin.manifest.author.is_empty() { String::new() } else { format!("by {} · ", plugin.manifest.author) };
                            hint(ui, &format!("{author}{} · {state}", what.join(", ")));
                            ui.horizontal(|ui| {
                                if ui.small_button("Show folder").clicked() {
                                    actions.push(PluginsAction::ShowFolder(id.clone()));
                                }
                                if enabled && plugin.runs_code() && ui.small_button("Restart").on_hover_text("Stop its code and start it again: how you reload it after editing").clicked() {
                                    actions.push(PluginsAction::Restart(id.clone()));
                                }
                                if plugin.origin == Origin::User && crate::icons::labelled(ui, crate::icons::Icon::Trash, "Remove…", false).clicked() {
                                    self.confirm_remove = Some(id.clone());
                                }
                            });
                            if let Some(log) = plugins.logs.get(&id).filter(|l| !l.is_empty()) {
                                egui::CollapsingHeader::new("Log").id_salt(("plugin-log", &id)).show(ui, |ui| {
                                    egui::ScrollArea::vertical().max_height(140.0).stick_to_bottom(true).show(ui, |ui| {
                                        for line in log {
                                            ui.label(egui::RichText::new(line).monospace().small());
                                        }
                                    });
                                });
                            }
                        });
                        ui.add_space(4.0);
                    }
                });
            });
        self.open = open;

        if let Some(id) = self.confirm_enable.clone() {
            let name = plugins.get(&id).map(|p| p.manifest.name.clone()).unwrap_or_else(|| id.clone());
            let mut answered = false;
            egui::Window::new("Turn on a plugin with code?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label(format!(
                        "`{name}` runs Python code on this computer, with your permissions: it can read and change files, and the scene, as you can. Turn on only plugins you trust."
                    ));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(egui::RichText::new("Turn it on").strong()).clicked() {
                            actions.push(PluginsAction::Enable(id.clone()));
                            answered = true;
                        }
                        if ui.button("Leave it off").clicked() {
                            answered = true;
                        }
                    });
                });
            if answered {
                self.confirm_enable = None;
            }
        }
        if let Some(id) = self.confirm_remove.clone() {
            let name = plugins.get(&id).map(|p| p.manifest.name.clone()).unwrap_or_else(|| id.clone());
            let mut answered = false;
            egui::Window::new("Remove plugin?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label(format!("`{name}` and its folder are deleted. Materials of its that projects use stay in those projects."));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(egui::RichText::new("Remove").color(crate::theme::p().danger).strong()).clicked() {
                            actions.push(PluginsAction::Remove(id.clone()));
                            answered = true;
                        }
                        if ui.button("Keep it").clicked() {
                            answered = true;
                        }
                    });
                });
            if answered {
                self.confirm_remove = None;
            }
        }
        actions
    }
}

/// The Plugins menu: each enabled plugin's commands, the others to turn
/// on, and the window.
pub fn menu(ui: &mut egui::Ui, plugins: &Plugins, window: &mut PluginsWindow) -> Option<PluginsAction> {
    let mut action = None;
    let mut any = false;
    for plugin in plugins.plugins.iter().filter(|p| plugins.is_enabled(&p.manifest.id) && !p.manifest.commands.is_empty()) {
        any = true;
        ui.menu_button(&plugin.manifest.name, |ui| {
            for command in &plugin.manifest.commands {
                let mut button = ui.button(&command.title);
                if !command.description.is_empty() {
                    button = button.on_hover_text(&command.description);
                }
                if button.clicked() {
                    action = Some(PluginsAction::Run { plugin: plugin.manifest.id.clone(), command: command.id.clone(), values: Vec::new() });
                    ui.close_menu();
                }
            }
        });
    }
    if any {
        ui.separator();
    }
    // The ones that are off, there to be turned on: on a new computer every
    // plugin is off, and one that is nowhere in the menu is one a person
    // thinks was never installed. Those Tequila ships turn on at a click;
    // one of the person's own with code asks first, as the window does.
    let off: Vec<_> = plugins.plugins.iter().filter(|p| !plugins.is_enabled(&p.manifest.id) && !p.manifest.commands.is_empty()).collect();
    if !off.is_empty() {
        ui.label(egui::RichText::new("Off · click to turn on").small().color(crate::theme::dim()));
        let python = crate::plugins::python().is_some();
        for plugin in off {
            let needs_python = plugin.runs_code() && !python;
            let mut item = ui.add_enabled(!needs_python, egui::Button::new(format!("Turn on {}", plugin.manifest.name)));
            if !plugin.manifest.description.is_empty() {
                item = item.on_hover_text(&plugin.manifest.description);
            }
            item = item.on_disabled_hover_text(
                "It runs Python code, and Python 3 is not on this computer: install it from python.org (or name it with TEQUILA_PYTHON), then turn it on.",
            );
            if item.clicked() {
                if plugin.origin == Origin::Bundled || !plugin.runs_code() {
                    action = Some(PluginsAction::Enable(plugin.manifest.id.clone()));
                } else {
                    window.ask_to_enable(&plugin.manifest.id);
                }
                ui.close_menu();
            }
        }
        ui.separator();
    }
    if crate::icons::labelled(ui, crate::icons::Icon::Puzzle, "Manage plugins…", false).clicked() {
        window.open = true;
        ui.close_menu();
    }
    action
}

/// Draw the open panels of the enabled plugins. A button gives its command
/// with the panel's values; closing a panel takes it away.
pub fn panels(ctx: &egui::Context, plugins: &mut Plugins) -> Vec<PluginsAction> {
    let mut actions = Vec::new();
    let mut closed = Vec::new();
    for (index, open) in plugins.panels.iter_mut().enumerate() {
        let Some(spec) = plugins
            .plugins
            .iter()
            .find(|p| p.manifest.id == open.plugin)
            .and_then(|p| p.manifest.panels.iter().find(|s| s.id == open.panel))
            .cloned()
        else {
            closed.push(index);
            continue;
        };
        let mut shown = true;
        egui::Window::new(&spec.title)
            .id(egui::Id::new(("plugin-panel", &open.plugin, &open.panel)))
            .open(&mut shown)
            .collapsible(true)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                for field in &spec.fields {
                    let slot = open.values.iter_mut().find(|(k, _)| *k == field.id);
                    match (&field.kind, slot) {
                        (FieldKind::Number { min, max, step, .. }, Some((_, value))) => {
                            let mut number = value.as_f32().map(f64::from).unwrap_or(*min);
                            ui.horizontal(|ui| {
                                ui.label(&field.label);
                                let mut drag = egui::DragValue::new(&mut number).range(*min..=*max);
                                if *step > 0.0 {
                                    drag = drag.speed(*step).fixed_decimals(if step.fract() == 0.0 { 0 } else { 2 });
                                } else {
                                    drag = drag.speed((max - min) / 200.0);
                                }
                                if ui.add(drag).changed() {
                                    *value = Value::Number(number);
                                }
                            });
                        }
                        (FieldKind::Text { lines: 1, .. }, Some((_, value))) => {
                            let mut text = value.as_str().unwrap_or_default().to_string();
                            ui.horizontal(|ui| {
                                ui.label(&field.label);
                                if ui.add(egui::TextEdit::singleline(&mut text).desired_width(180.0)).changed() {
                                    *value = Value::String(text);
                                }
                            });
                        }
                        (FieldKind::Text { lines, .. }, Some((_, value))) => {
                            let mut text = value.as_str().unwrap_or_default().to_string();
                            ui.label(&field.label);
                            if ui.add(egui::TextEdit::multiline(&mut text).desired_rows(*lines).desired_width(f32::INFINITY)).changed() {
                                *value = Value::String(text);
                            }
                        }
                        (FieldKind::Toggle { .. }, Some((_, value))) => {
                            let mut on = value.as_bool().unwrap_or(false);
                            if ui.add(crate::field::Checkbox::new(&mut on, &field.label)).changed() {
                                *value = Value::Bool(on);
                            }
                        }
                        (FieldKind::Choice { options, .. }, Some((_, value))) => {
                            let mut chosen = value.as_str().unwrap_or_default().to_string();
                            ui.horizontal(|ui| {
                                ui.label(&field.label);
                                egui::ComboBox::from_id_salt(("plugin-choice", &open.plugin, &field.id)).selected_text(&chosen).show_ui(ui, |ui| {
                                    for option in options {
                                        ui.selectable_value(&mut chosen, option.clone(), option);
                                    }
                                });
                            });
                            if value.as_str() != Some(chosen.as_str()) {
                                *value = Value::String(chosen);
                            }
                        }
                        (FieldKind::Colour { .. }, Some((_, value))) => {
                            let channels: Vec<f32> = value.as_array().iter().filter_map(|c| c.as_f32()).collect();
                            let mut rgb = if channels.len() == 3 { [channels[0], channels[1], channels[2]] } else { [0.8, 0.8, 0.8] };
                            ui.horizontal(|ui| {
                                ui.label(&field.label);
                                let mut srgb = rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
                                if ui.color_edit_button_srgb(&mut srgb).changed() {
                                    rgb = srgb.map(|c| c as f32 / 255.0);
                                    *value = Value::Array(rgb.iter().map(|c| Value::Number(*c as f64)).collect());
                                }
                            });
                        }
                        (FieldKind::Label { .. }, Some((_, value))) => {
                            hint(ui, value.as_str().unwrap_or_default());
                        }
                        (FieldKind::Path { folder, save, extensions, .. }, Some((_, value))) => {
                            let mut path = value.as_str().unwrap_or_default().to_string();
                            ui.label(&field.label);
                            ui.horizontal(|ui| {
                                let edited = ui.add(egui::TextEdit::singleline(&mut path).desired_width(200.0).hint_text(if *folder { "a folder" } else { "a file" })).changed();
                                let picked = ui.button("Browse…").clicked().then(|| {
                                    let mut dialog = rfd::FileDialog::new().set_title(&field.label);
                                    if !extensions.is_empty() {
                                        dialog = dialog.add_filter(&field.label, extensions);
                                    }
                                    match (*folder, *save) {
                                        (true, _) => dialog.pick_folder(),
                                        (false, true) => dialog.save_file(),
                                        (false, false) => dialog.pick_file(),
                                    }
                                });
                                if let Some(Some(chosen)) = &picked {
                                    path = chosen.display().to_string();
                                }
                                if edited || picked.is_some() {
                                    *value = Value::String(path);
                                }
                            });
                        }
                        (FieldKind::Button { command }, _) => {
                            if ui.button(&field.label).clicked() {
                                actions.push(PluginsAction::Run { plugin: open.plugin.clone(), command: command.clone(), values: open.values.clone() });
                            }
                        }
                        _ => {}
                    }
                }
            });
        if !shown {
            closed.push(index);
        }
    }
    for index in closed.into_iter().rev() {
        plugins.panels.remove(index);
    }
    actions
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().color(crate::theme::dim()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window with plugins in it, and a plugin's panel open, lay out:
    /// the panel draws every kind of field, and nothing runs without a click.
    #[test]
    fn the_plugins_window_and_a_panel_lay_out() {
        let root = std::env::temp_dir().join(format!("tequila-plugins-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("every-field");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            r#"{"id":"every.field","name":"Every Field","main":"main.py","commands":[{"id":"open","panel":"p"}],
               "panels":[{"id":"p","title":"Every field","fields":[
                 {"id":"n","type":"number","label":"Count","min":1,"max":10,"step":1,"default":3},
                 {"id":"t","type":"text","label":"Name","default":"x"},
                 {"id":"b","type":"toggle","label":"On"},
                 {"id":"c","type":"choice","label":"Kind","options":["a","b"]},
                 {"id":"k","type":"colour","label":"Colour","default":[0.5,0.2,0.1]},
                 {"id":"l","type":"label","text":"Hello"},
                 {"id":"m","type":"text","label":"Notes","lines":4,"default":"one\ntwo"},
                 {"id":"f","type":"file","label":"Rules","extensions":["csv"]},
                 {"id":"s","type":"file","label":"Save to","save":true,"default":"C:/out.png"},
                 {"id":"d","type":"folder","label":"Into"},
                 {"id":"go","type":"button","label":"Go","command":"go"}]}]}"#,
        )
        .unwrap();
        let mut plugins = Plugins::load_from(vec![(root.clone(), Origin::User)], root.join("plugins.json"));
        assert!(plugins.problems.is_empty(), "{:?}", plugins.problems);
        plugins.open_panel("every.field", "p");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut window = PluginsWindow { open: true, ..Default::default() };
        // And the question a script's or an AI app's request to turn it on asks.
        window.ask_to_enable("every.field");
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
                assert!(window.show(ctx, &plugins).is_empty());
                assert!(panels(ctx, &mut plugins).is_empty());
            });
        }
        assert!(ctx.memory(|m| m.area_rect(egui::Id::new("Plugins"))).is_some(), "the window is not on screen");
        let panel = ctx.memory(|m| m.area_rect(egui::Id::new(("plugin-panel", "every.field", "p")))).expect("the panel is not on screen");
        assert!(panel.height() > 150.0, "the panel's fields are missing: {panel:?}");
        assert_eq!(plugins.panels.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}

impl crate::TequilaApp {
    /// The plugins' share of a frame: start the enabled ones, take in what
    /// they said, tell them of a changed selection, offer their materials
    /// to the library, and draw the window and their panels.
    pub(crate) fn plugins_frame(&mut self, ctx: &egui::Context) {
        let discovery = self.channel.as_ref().map(|channel| {
            let record = tequila_scripting::server::instances_dir(&tequila_scripting::server::discovery_path()).join(format!("{}.json", channel.port()));
            (record, std::process::id())
        });
        self.plugins.start_enabled(discovery.as_ref().map(|(path, pid)| (path.as_path(), *pid)));
        for (name, message, error) in self.plugins.poll() {
            let line = format!("{name}: {message}");
            if error {
                self.toasts.warn(line);
            } else {
                self.toasts.info(line);
            }
        }
        let selection: Vec<u64> = self.project.selection.nodes().map(|n| n.0).collect();
        if selection != self.plugin_selection {
            self.plugin_selection = selection.clone();
            let mut data = tequila_format::json_write::Object::new();
            data.set("nodes", Value::Array(selection.iter().map(|n| Value::Number(*n as f64)).collect()));
            self.plugins.broadcast("selection", &data.build());
        }
        self.library.set_plugin_folders(self.plugins.material_folders());
        if !self.plugins.hosts.is_empty() {
            // What a plugin says arrives on its own time.
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        let mut actions = self.plugins_window.show(ctx, &self.plugins);
        actions.extend(panels(ctx, &mut self.plugins));
        for action in actions {
            self.plugin_action(action);
        }
    }

    /// Tell the plugins that asked: the project was saved, a render
    /// finished.
    pub(crate) fn plugin_event(&mut self, event: &str, path: &std::path::Path) {
        let mut data = tequila_format::json_write::Object::new();
        data.set("path", path.display().to_string());
        self.plugins.broadcast(event, &data.build());
    }

    /// Every node now in the scene: taken before an import, to tell the
    /// plugins what it added.
    pub(crate) fn node_ids(&self) -> std::collections::BTreeSet<tequila_scene::tree::NodeId> {
        self.project.document().tree.iter().map(|n| n.id).collect()
    }

    /// Tell the plugins that asked: `path` was imported, adding every node
    /// not in `before` - all of them, and the parts among them.
    pub(crate) fn plugin_imported(&mut self, path: &std::path::Path, before: &std::collections::BTreeSet<tequila_scene::tree::NodeId>) {
        let document = self.project.document();
        let added: Vec<&tequila_scene::SceneNode> = document.tree.iter().filter(|n| !before.contains(&n.id)).collect();
        let ids = |parts: bool| Value::Array(added.iter().filter(|n| !parts || n.mesh().is_some()).map(|n| Value::Number(n.id.0 as f64)).collect());
        let mut data = tequila_format::json_write::Object::new();
        data.set("path", path.display().to_string()).set("nodes", ids(false)).set("parts", ids(true));
        let data = data.build();
        self.plugins.broadcast("imported", &data);
    }

    pub(crate) fn plugin_action(&mut self, action: PluginsAction) {
        let folder = self.plugins.user_folder();
        match action {
            PluginsAction::Install => {
                let Some(zip) = rfd::FileDialog::new().set_title("Install a plugin").add_filter("Plugin", &["zip"]).pick_file() else { return };
                match crate::plugins::install_zip(&zip, &folder, false) {
                    Ok(installed) => {
                        self.plugins.reload();
                        let name = self.plugins.plugins.iter().find(|p| p.folder == installed).map(|p| p.manifest.name.clone()).unwrap_or_default();
                        self.toasts.info(format!("Installed `{name}`; turn it on in the Plugins window"));
                    }
                    Err(error) => self.toasts.warn(error),
                }
            }
            PluginsAction::New(name) => match crate::plugins::create_from_template(&folder, &name) {
                Ok(made) => {
                    self.plugins.reload();
                    crate::library_tab::open_folder(&made);
                    self.toasts.info(format!("Made `{name}` from the template; turn it on to try it, and edit its main.py"));
                }
                Err(error) => self.toasts.warn(error),
            },
            PluginsAction::OpenFolder => crate::library_tab::open_folder(&folder),
            PluginsAction::Reload => self.plugins.reload(),
            PluginsAction::Enable(id) => {
                if let Err(error) = self.plugins.set_enabled(&id, true) {
                    self.toasts.warn(error);
                }
            }
            PluginsAction::Disable(id) => {
                if let Err(error) = self.plugins.set_enabled(&id, false) {
                    self.toasts.warn(error);
                }
            }
            PluginsAction::Restart(id) => self.plugins.restart(&id),
            PluginsAction::Remove(id) => {
                let Some(plugin) = self.plugins.get(&id).cloned() else { return };
                let _ = self.plugins.set_enabled(&id, false);
                match crate::plugins::remove(&plugin) {
                    Ok(()) => {
                        self.plugins.reload();
                        self.toasts.info(format!("Removed `{}`", plugin.manifest.name));
                    }
                    Err(error) => self.toasts.warn(error),
                }
            }
            PluginsAction::ShowFolder(id) => {
                if let Some(folder) = self.plugins.get(&id).map(|p| p.folder.clone()) {
                    crate::library_tab::open_folder(&folder);
                }
            }
            PluginsAction::Run { plugin, command, values } => {
                if let Err(error) = self.plugins.run(&plugin, &command, &values) {
                    self.toasts.warn(error);
                }
            }
        }
    }
}
