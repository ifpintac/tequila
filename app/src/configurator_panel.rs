//! The Configurator wizard and its picker (`TEQUILA_SPEC.md` §1.13).
//!
//! The wizard is four steps in the Studios tab, in the order a product is
//! set up: the **options** a customer chooses between and what each choice
//! switches; the **rules** that keep impossible products out; the
//! **layout** of the picker; and **try it**, the picker itself working on
//! the scene, with the way into Presentation mode and the batch render.
//!
//! The picker is the same widget in the wizard and in Presentation mode, so
//! what is tried is what is presented. A choice that the rest of the
//! configuration rules out is still clickable: picking it moves the other
//! options to the nearest configuration that allows it, and its tooltip says
//! what will move. A choice nothing allows is drawn struck through.

use crate::project::Project;
use crate::theme;
use eframe::egui;
use tequila_scene::configurator::{Choice, ConfigOption, Configurator, Drives, Placement, Rule, RuleKind, Style};
use tequila_scene::document::SceneDocument;
use tequila_scene::Command;

/// What the wizard asks the application to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Present,
    RenderAll,
    ExportWeb,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Step {
    #[default]
    Options,
    Rules,
    Layout,
    Try,
}

#[derive(Debug, Default)]
pub struct Wizard {
    step: Step,
}

/// The names of what an option can switch to, by target index.
fn targets(document: &SceneDocument, drives: Drives) -> Vec<String> {
    match drives {
        Drives::Material { multi_material } => document
            .multi_materials
            .iter()
            .find(|m| m.id == multi_material)
            .map(|m| {
                m.variants
                    .iter()
                    .enumerate()
                    .map(|(i, key)| document.materials.get(*key).map(|d| d.name.clone()).unwrap_or_else(|| format!("Variant {}", i + 1)))
                    .collect()
            })
            .unwrap_or_default(),
        Drives::ModelSet => document.model_sets.sets.iter().map(|s| s.name.clone()).collect(),
        Drives::Studio => document.studios.iter().map(|s| s.name.clone()).collect(),
    }
}

/// What an option switches, in words.
fn switches(document: &SceneDocument, drives: Drives) -> String {
    match drives {
        Drives::Material { multi_material } => match document.multi_materials.iter().find(|m| m.id == multi_material) {
            Some(m) => format!("Switches the variants of “{}”", m.name),
            None => "Switches a Multi-Material that was deleted".into(),
        },
        Drives::ModelSet => "Switches the model set".into(),
        Drives::Studio => "Switches the Studio: camera, lighting and look".into(),
    }
}

/// A choice's swatch on screen: its own colour, or its variant's base
/// colour, or `None` for an option that switches something colourless.
pub fn swatch(document: &SceneDocument, option: &ConfigOption, choice: &Choice) -> Option<egui::Color32> {
    let rgb = match (choice.swatch, option.drives) {
        (Some(rgb), _) => rgb,
        (None, Drives::Material { multi_material }) => {
            let multi = document.multi_materials.iter().find(|m| m.id == multi_material)?;
            let key = multi.variants.get(choice.target)?;
            crate::color_picker::to_picker(document.materials.get(*key)?.material().base_color)
        }
        (None, _) => return None,
    };
    Some(egui::Color32::from(egui::Rgba::from_rgb(rgb[0], rgb[1], rgb[2])))
}

impl Wizard {
    pub fn show(&mut self, ui: &mut egui::Ui, project: &mut Project) -> Option<Action> {
        ui.small(
            "Turns the variants, model sets and Studios above into a product \
             configurator: the choices a customer makes, the rules between them, \
             and a picker to present full screen or render every combination from.",
        );
        let original = project.document().configurator.clone();
        if original.is_empty() {
            let draft = Configurator::draft(project.document());
            if draft.is_empty() {
                ui.weak("Make a Multi-Material, two model sets or two Studios first: those are what a configurator's choices switch.");
                return None;
            }
            if ui
                .button("Build a configurator")
                .on_hover_text("Starts with one option for each Multi-Material, and for the model sets and Studios when there are several.")
                .clicked()
            {
                let result = project.apply(Command::SetConfigurator { configurator: Box::new(draft) });
                if let Err(error) = result {
                    project.status = error.to_string();
                }
                self.step = Step::Options;
            }
            return None;
        }

        let mut step = self.step;
        theme::segmented(ui, &mut step, &[(Step::Options, "Options"), (Step::Rules, "Rules"), (Step::Layout, "Layout"), (Step::Try, "Try it")]);
        self.step = step;
        ui.add_space(4.0);

        let mut edited = original.clone();
        let mut action = None;
        match self.step {
            Step::Options => options_step(ui, project.document(), &mut edited),
            Step::Rules => rules_step(ui, project.document(), &mut edited),
            Step::Layout => layout_step(ui, &mut edited),
            Step::Try => action = try_step(ui, project),
        }
        if edited != original {
            // An edit can leave the shown configuration breaking a new rule
            // or pointing at a removed choice: settle it on the nearest one
            // that works, so the configurator is never left showing nothing.
            if let Some(settled) = edited.settled() {
                edited.current = settled;
            }
            if let Err(error) = project.apply(Command::SetConfigurator { configurator: Box::new(edited) }) {
                project.status = error.to_string();
            }
        }
        action
    }
}

fn options_step(ui: &mut egui::Ui, document: &SceneDocument, c: &mut Configurator) {
    let mut remove = None;
    let mut swap = None;
    let count = c.options.len();
    for index in 0..count {
        let option = &mut c.options[index];
        let names = targets(document, option.drives);
        ui.push_id(("config-option", option.id), |ui| {
            crate::field::Fold::new(format!("{} · {}", option.name, plural(option.choices.len(), "choice")))
                .id_salt(("config-option-fold", option.id))
                .default_open(count <= 3)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut option.name).desired_width(ui.available_width()));
                    });
                    ui.label(egui::RichText::new(switches(document, option.drives)).small().color(theme::p().text_dim));
                    let mut style = option.style;
                    let styles: Vec<(Style, &str)> = Style::ALL.iter().map(|s| (*s, s.label())).collect();
                    theme::segmented(ui, &mut style, &styles);
                    option.style = style;

                    let mut drop_choice = None;
                    let is_material = matches!(option.drives, Drives::Material { .. });
                    for j in 0..option.choices.len() {
                        ui.horizontal(|ui| {
                            if is_material {
                                let shown = swatch(document, option, &option.choices[j]);
                                let mut colour = shown.map(|c| crate::color_picker::from_picker({
                                    let rgba = egui::Rgba::from(c);
                                    [rgba.r(), rgba.g(), rgba.b()]
                                }));
                                if let Some(colour) = colour.as_mut() {
                                    if crate::color_picker::edit(ui, colour)
                                        .on_hover_text("The swatch the customer sees. Taken from the variant until you pick one.")
                                        .changed()
                                    {
                                        option.choices[j].swatch = Some(crate::color_picker::to_picker(*colour));
                                    }
                                }
                            }
                            let choice = &mut option.choices[j];
                            let width = (ui.available_width() - 150.0).max(60.0);
                            ui.add(egui::TextEdit::singleline(&mut choice.label).desired_width(width).hint_text("Label"));
                            let current = names.get(choice.target).cloned().unwrap_or_else(|| "(missing)".into());
                            egui::ComboBox::from_id_salt(("config-target", j))
                                .width(100.0)
                                .selected_text(current)
                                .show_ui(ui, |ui| {
                                    for (t, name) in names.iter().enumerate() {
                                        ui.selectable_value(&mut choice.target, t, name);
                                    }
                                })
                                .response
                                .on_hover_text("What this choice switches to");
                            if crate::icons::button(ui, crate::icons::Icon::Close, false, 20.0, egui::Sense::click())
                                .on_hover_text("Remove this choice")
                                .clicked()
                            {
                                drop_choice = Some(j);
                            }
                        });
                    }
                    if let Some(j) = drop_choice {
                        option.choices.remove(j);
                    }
                    ui.horizontal(|ui| {
                        let unused = (0..names.len()).find(|t| !option.choices.iter().any(|c| c.target == *t));
                        if ui
                            .add_enabled(!names.is_empty(), egui::Button::new("+ Choice"))
                            .on_hover_text("Add a choice for the next thing it can switch to")
                            .clicked()
                        {
                            let target = unused.unwrap_or(0);
                            option.choices.push(Choice { label: names.get(target).cloned().unwrap_or_default(), target, swatch: None });
                        }
                        if ui.small_button("Up").clicked() && index > 0 {
                            swap = Some((index, index - 1));
                        }
                        if ui.small_button("Down").clicked() && index + 1 < count {
                            swap = Some((index, index + 1));
                        }
                        if ui.small_button("Remove option").clicked() {
                            remove = Some(index);
                        }
                    });
                });
        });
    }
    if let Some(index) = remove {
        let id = c.options[index].id;
        c.options.remove(index);
        c.rules.retain(|r| r.option != id && r.other != id);
        if index < c.current.len() {
            c.current.remove(index);
        }
    }
    if let Some((a, b)) = swap {
        c.options.swap(a, b);
        if a < c.current.len() && b < c.current.len() {
            c.current.swap(a, b);
        }
    }
    // Choices removed or added: keep `current` one per option.
    c.current.resize(c.options.len(), 0);

    ui.add_space(4.0);
    ui.menu_button("+ Option", |ui| {
        let add = |c: &mut Configurator, name: String, drives: Drives| {
            let names = targets(document, drives);
            let id = c.next_id();
            c.options.push(ConfigOption {
                id,
                name,
                drives,
                choices: names.iter().enumerate().map(|(i, n)| Choice { label: n.clone(), target: i, swatch: None }).collect(),
                style: if matches!(drives, Drives::Material { .. }) { Style::Swatches } else { Style::Buttons },
            });
            c.current.push(0);
        };
        for multi in &document.multi_materials {
            if ui.button(format!("Variants of “{}”", multi.name)).clicked() {
                add(c, multi.name.clone(), Drives::Material { multi_material: multi.id });
                ui.close_menu();
            }
        }
        if ui.add_enabled(!document.model_sets.sets.is_empty(), egui::Button::new("Model sets")).clicked() {
            add(c, "Model".into(), Drives::ModelSet);
            ui.close_menu();
        }
        if ui.add_enabled(!document.studios.is_empty(), egui::Button::new("Studios")).clicked() {
            add(c, "View".into(), Drives::Studio);
            ui.close_menu();
        }
    });
    problems(ui, document, c);
}

fn rules_step(ui: &mut egui::Ui, document: &SceneDocument, c: &mut Configurator) {
    let valid = c.combinations().len();
    ui.label(
        egui::RichText::new(format!(
            "{} of {} combinations are valid",
            if valid >= tequila_scene::configurator::MOST_COMBINATIONS { format!("{valid}+") } else { valid.to_string() },
            c.product()
        ))
        .color(theme::p().text_dim),
    );
    if c.options.len() < 2 {
        ui.weak("Rules are between two options; add a second option first.");
        return;
    }
    ui.small("When a customer picks the first choice, the second is ruled out, or required.");
    let options: Vec<(u64, String, Vec<String>)> =
        c.options.iter().map(|o| (o.id, o.name.clone(), o.choices.iter().map(|c| c.label.clone()).collect())).collect();
    let mut remove = None;
    for (index, rule) in c.rules.iter_mut().enumerate() {
        ui.push_id(("config-rule", index), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("When");
                pick_choice(ui, "first", &options, &mut rule.option, &mut rule.choice);
                egui::ComboBox::from_id_salt("kind")
                    .width(80.0)
                    .selected_text(match rule.kind {
                        RuleKind::Excludes => "rules out",
                        RuleKind::Requires => "requires",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut rule.kind, RuleKind::Excludes, "rules out");
                        ui.selectable_value(&mut rule.kind, RuleKind::Requires, "requires");
                    });
                pick_choice(ui, "second", &options, &mut rule.other, &mut rule.other_choice);
                if crate::icons::button(ui, crate::icons::Icon::Close, false, 20.0, egui::Sense::click())
                    .on_hover_text("Remove this rule")
                    .clicked()
                {
                    remove = Some(index);
                }
            });
        });
        ui.separator();
    }
    if let Some(index) = remove {
        c.rules.remove(index);
    }
    if ui.button("+ Rule").clicked() {
        let (a, b) = (&c.options[0], &c.options[1]);
        c.rules.push(Rule { option: a.id, choice: 0, kind: RuleKind::Excludes, other: b.id, other_choice: b.choices.len().saturating_sub(1) });
    }
    problems(ui, document, c);
}

/// An option and one of its choices, as two drop-downs.
fn pick_choice(ui: &mut egui::Ui, salt: &str, options: &[(u64, String, Vec<String>)], option: &mut u64, choice: &mut usize) {
    let name = options.iter().find(|o| o.0 == *option).map(|o| o.1.clone()).unwrap_or_else(|| "(removed)".into());
    egui::ComboBox::from_id_salt((salt, "option")).width(80.0).selected_text(name).show_ui(ui, |ui| {
        for (id, name, _) in options {
            if ui.selectable_value(option, *id, name).clicked() {
                *choice = 0;
            }
        }
    });
    let labels = options.iter().find(|o| o.0 == *option).map(|o| o.2.clone()).unwrap_or_default();
    egui::ComboBox::from_id_salt((salt, "choice"))
        .width(80.0)
        .selected_text(labels.get(*choice).cloned().unwrap_or_else(|| "(none)".into()))
        .show_ui(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                ui.selectable_value(choice, i, label);
            }
        });
}

fn layout_step(ui: &mut egui::Ui, c: &mut Configurator) {
    ui.horizontal(|ui| {
        ui.label("Title");
        ui.add(egui::TextEdit::singleline(&mut c.title).desired_width(ui.available_width()).hint_text("The product's name"));
    });
    ui.horizontal(|ui| {
        ui.label("Subtitle");
        ui.add(egui::TextEdit::singleline(&mut c.subtitle).desired_width(ui.available_width()).hint_text("A line under it"));
    });
    ui.label(egui::RichText::new("Picker").color(theme::p().text_dim));
    let mut placement = c.placement;
    let places: Vec<(Placement, &str)> = Placement::ALL.iter().map(|p| (*p, p.label())).collect();
    theme::segmented(ui, &mut placement, &places);
    c.placement = placement;
    ui.small("Each option's look - swatches, buttons or a list - is set with the option.");
    ui.add_space(8.0);
    if ui.small_button("Remove the configurator").on_hover_text("Undo brings it back.").clicked() {
        *c = Configurator::default();
    }
}

fn try_step(ui: &mut egui::Ui, project: &mut Project) -> Option<Action> {
    let configurator = project.document().configurator.clone();
    let mut action = None;
    if let Some(choices) = picker(ui, project.document(), &configurator, false) {
        if let Err(error) = project.apply(Command::Configure { choices }) {
            project.status = error.to_string();
        }
    }
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Present full screen").on_hover_text("The picker over the live view, for a showroom or a call. Esc leaves.").clicked() {
            action = Some(Action::Present);
        }
        let count = configurator.combinations().len();
        if ui
            .add_enabled(count > 0, egui::Button::new(format!("Render all {count}…")))
            .on_hover_text("Queue a render of every valid combination, named by its choices")
            .clicked()
        {
            action = Some(Action::RenderAll);
        }
        if ui
            .button("Export for the web…")
            .on_hover_text("A folder with the configurator in it - live 3D, photos of every combination, or both - for any web host or straight off the disk")
            .clicked()
        {
            action = Some(Action::ExportWeb);
        }
    });
    action
}

fn problems(ui: &mut egui::Ui, document: &SceneDocument, c: &Configurator) {
    for problem in c.problems(document) {
        ui.label(egui::RichText::new(format!("⚠ {problem}")).small().color(theme::p().danger));
    }
}

/// The customer's picker: every option, its choices drawn in its style.
/// Returns the configuration a click asks for, already moved to satisfy the
/// rules; the caller applies it. `large` draws it for a full screen.
pub fn picker(ui: &mut egui::Ui, document: &SceneDocument, c: &Configurator, large: bool) -> Option<Vec<usize>> {
    picker_laid_out(ui, document, c, large, false)
}

/// [`picker`], with the options side by side in columns when `columns` -
/// for a picker along the bottom of a presentation.
pub fn picker_laid_out(ui: &mut egui::Ui, document: &SceneDocument, c: &Configurator, large: bool, columns: bool) -> Option<Vec<usize>> {
    let current = c.settled().unwrap_or_else(|| c.current.clone());
    let mut picked = None;
    if columns {
        let width = (ui.available_width() / c.options.len().max(1) as f32 - 16.0).max(140.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            for i in 0..c.options.len() {
                ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(width);
                    if let Some(p) = option_picker(ui, document, c, &current, i, large) {
                        picked = Some(p);
                    }
                });
            }
        });
    } else {
        for i in 0..c.options.len() {
            if let Some(p) = option_picker(ui, document, c, &current, i, large) {
                picked = Some(p);
            }
            ui.add_space(if large { 10.0 } else { 6.0 });
        }
    }
    picked
}

/// One option of the picker: its name and what is chosen, then its
/// choices in its style.
fn option_picker(ui: &mut egui::Ui, document: &SceneDocument, c: &Configurator, current: &[usize], i: usize, large: bool) -> Option<Vec<usize>> {
    let mut picked = None;
    let size = if large { 34.0 } else { 26.0 };
    let option = &c.options[i];
    {
        let chosen = current.get(i).copied().unwrap_or(0);
        let impossible = c.impossible(i);
        ui.push_id(("picker-option", option.id), |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&option.name).strong().size(if large { 15.0 } else { 13.0 }));
                if let Some(choice) = option.choices.get(chosen) {
                    ui.label(egui::RichText::new(&choice.label).color(theme::p().text_dim).size(if large { 15.0 } else { 13.0 }));
                }
            });
            // What picking `j` would also change, for its tooltip.
            let consequence = |j: usize| -> Option<String> {
                let next = c.resolve(current, i, j)?;
                let moved: Vec<String> = c
                    .options
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| *k != i && next.get(*k) != current.get(*k))
                    .map(|(k, o)| format!("{} to {}", o.name, o.choices.get(next[k]).map(|c| c.label.as_str()).unwrap_or("?")))
                    .collect();
                (!moved.is_empty()).then(|| format!("Also changes {}", moved.join(", ")))
            };
            let mut click = |j: usize| {
                if j != chosen && !impossible.get(j).copied().unwrap_or(false) {
                    picked = c.resolve(current, i, j);
                }
            };
            match option.style {
                Style::Swatches => {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                        for (j, choice) in option.choices.iter().enumerate() {
                            let colour = swatch(document, option, choice).unwrap_or(theme::p().line_strong);
                            let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click());
                            let painter = ui.painter();
                            let centre = rect.center();
                            let radius = size * 0.5 - 3.0;
                            painter.circle_filled(centre, radius, colour);
                            painter.circle_stroke(centre, radius, egui::Stroke::new(1.0_f32, theme::p().line_strong));
                            if j == chosen {
                                painter.circle_stroke(centre, radius + 2.5, egui::Stroke::new(2.0_f32, theme::p().accent));
                            } else if response.hovered() {
                                painter.circle_stroke(centre, radius + 2.5, egui::Stroke::new(1.5_f32, theme::p().text_faint));
                            }
                            let blocked = impossible.get(j).copied().unwrap_or(false);
                            if blocked {
                                let d = radius * 0.7;
                                painter.line_segment([centre + egui::vec2(-d, d), centre + egui::vec2(d, -d)], egui::Stroke::new(2.0_f32, theme::p().text));
                            }
                            let tip = match (blocked, consequence(j)) {
                                (true, _) => format!("{} - not available", choice.label),
                                (false, Some(more)) => format!("{}\n{more}", choice.label),
                                (false, None) => choice.label.clone(),
                            };
                            if response.on_hover_text(tip).clicked() {
                                click(j);
                            }
                        }
                    });
                }
                Style::Buttons => {
                    ui.horizontal_wrapped(|ui| {
                        for (j, choice) in option.choices.iter().enumerate() {
                            let blocked = impossible.get(j).copied().unwrap_or(false);
                            let text = egui::RichText::new(&choice.label).size(if large { 15.0 } else { 13.0 });
                            let text = if blocked { text.strikethrough() } else { text };
                            let response = ui.add_enabled(!blocked, egui::SelectableLabel::new(j == chosen, text));
                            let response = match consequence(j) {
                                Some(more) => response.on_hover_text(more),
                                None => response,
                            };
                            if response.clicked() {
                                click(j);
                            }
                        }
                    });
                }
                Style::List => {
                    let mut selected = chosen;
                    egui::ComboBox::from_id_salt("picker-list")
                        .width(if large { 220.0 } else { 160.0 })
                        .selected_text(option.choices.get(chosen).map(|c| c.label.clone()).unwrap_or_default())
                        .show_ui(ui, |ui| {
                            for (j, choice) in option.choices.iter().enumerate() {
                                let blocked = impossible.get(j).copied().unwrap_or(false);
                                ui.add_enabled_ui(!blocked, |ui| ui.selectable_value(&mut selected, j, &choice.label));
                            }
                        });
                    if selected != chosen {
                        click(selected);
                    }
                }
            }
        });
    }
    picked
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scene::studio::MultiMaterial;
    use tequila_scene::tree::NodeKind;

    /// The demo with a two-variant Multi-Material on its part and a drafted
    /// configurator over it.
    fn project() -> Project {
        let mut project = Project::with_document(
            tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2),
            std::sync::Arc::new(tequila_scene::demo::studio_environment()),
        );
        let part = project.document().tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| n.id).unwrap();
        let first = project.document().tree.get(part).unwrap().material.unwrap();
        project.apply(Command::UnlinkMaterial { node: part }).unwrap();
        let second = project.document().tree.get(part).unwrap().material.unwrap();
        project
            .apply(Command::SetMultiMaterials {
                multi_materials: vec![MultiMaterial { id: 9, name: "Body".into(), variants: vec![first, second], active: 0, parts: vec![part] }],
            })
            .unwrap();
        let draft = Configurator::draft(project.document());
        project.apply(Command::SetConfigurator { configurator: Box::new(draft) }).unwrap();
        project
    }

    fn frame(ctx: &egui::Context, add: impl FnMut(&mut egui::Ui)) {
        let mut add = add;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 900.0))),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| add(ui));
        });
    }

    /// Every step draws, with a configurator that works and with one whose
    /// Multi-Material has since been deleted, and drawing changes nothing.
    #[test]
    fn every_step_draws_without_touching_the_document() {
        for broken in [false, true] {
            let mut project = project();
            if broken {
                project.apply(Command::SetMultiMaterials { multi_materials: Vec::new() }).unwrap();
            }
            let before = project.document().configurator.clone();
            let ctx = egui::Context::default();
            for step in [Step::Options, Step::Rules, Step::Layout, Step::Try] {
                let mut wizard = Wizard { step };
                for _ in 0..2 {
                    frame(&ctx, |ui| {
                        assert_eq!(wizard.show(ui, &mut project), None);
                    });
                }
            }
            assert_eq!(project.document().configurator, before, "drawing edited the configurator");
            assert_eq!(project.document().configurator.problems(project.document()).is_empty(), !broken);
        }
    }

    /// A choice's swatch is its variant's colour until one is picked.
    #[test]
    fn a_swatch_comes_from_its_variant() {
        let project = project();
        let c = &project.document().configurator;
        let option = &c.options[0];
        let from_variant = swatch(project.document(), option, &option.choices[0]).expect("a colour");
        let mut own = option.choices[0].clone();
        own.swatch = Some([0.0, 0.0, 1.0]);
        assert_eq!(swatch(project.document(), option, &own), Some(egui::Color32::from_rgb(0, 0, 255)));
        assert_ne!(from_variant, egui::Color32::from_rgb(0, 0, 255));
    }
}
