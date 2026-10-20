//! The Material tab (`docs/04-UX-SPEC.md` §3).
//!
//! Every control here is generated, not written. The panel asks the material's
//! catalog type which parameters it exposes, asks each parameter what kind of
//! control it wants and what bounds it should offer, and builds the row. There
//! is no `match` on parameter names anywhere in this file.
//!
//! That is worth the indirection because the alternative drifts. Thirty-seven
//! parameters listed by hand here, again in the undo journal, again in the file
//! format and again in the Python API is four lists that agree only as long as
//! nobody is in a hurry. They agree here because they are all reading
//! [`Param`], and a test in `tequila-materials` fails if the table falls
//! behind the parameter block.
//!
//! The other thing this panel has to get right is §1.4's linking model, which
//! `TEQUILA_SPEC.md` calls "the single most commonly misunderstood part" of
//! the product. Editing a material edits it for **every** part that shares it.
//! That is the feature, and a panel that lets someone discover it by accident
//! has failed — so the reference count is stated on the face of the panel
//! rather than hidden behind a hover, and Unlink is next to it.

use eframe::egui;
use tequila_core::spectrum::Spectrum;
use tequila_materials::fresnel::metals;
use tequila_materials::material_type::{MaterialType, Param, ParamUi, ParamValue};
use tequila_scene::command::MaterialParameter;
#[cfg(test)]
use tequila_scene::selection::SelectionMode;
use tequila_scene::textures::TextureSource;
use tequila_scene::tree::{MaterialKey, NodeId};
use tequila_scene::Command;
use tequila_texture::texture::MapRole;

use crate::project::Project;

/// A panel with nothing to show, saying what to do about it.
///
/// A bare "nothing selected" tells somebody the state they are already looking
/// at. The second line is the part that helps.
fn empty_state(ui: &mut egui::Ui, icon: crate::icons::Icon, title: &str, hint: &str) {
    crate::theme::empty_state(ui, icon, crate::theme::hue(crate::theme::Hue::Pink), title, hint);
}

#[derive(Default)]
pub struct MaterialTab {
    /// A label whose viewport placement the user just asked to start or stop.
    ///
    /// Drained by the app each frame and handed to the viewport. The panel
    /// cannot start the mode itself — the mode lives where the clicking
    /// happens — and passing a whole app handle down into a material panel to
    /// let it do so would be a far worse trade than one field.
    pub place_label_request: Option<(MaterialKey, usize)>,
    /// The material whose graph the person asked to open. Drained by the
    /// app, which owns the editor window.
    pub open_graph: Option<MaterialKey>,
    /// Which label is being placed right now, so its button can show as held
    /// down. Set by the app from the viewport's own state.
    ///
    /// The material travels with the index because the index alone is not an
    /// identity: label 0 of the part being placed and label 0 of whichever
    /// material the panel is showing now are different labels, and comparing
    /// indices would light the button on the wrong one.
    pub placing_label: Option<(MaterialKey, usize)>,
    /// Show every parameter rather than the type's exposed set.
    ///
    /// The exposed set is the right default — §3.1 gives Plastic six controls
    /// out of thirty-seven, and showing the other thirty-one would bury them.
    /// But hiding a parameter is not the same as it not existing, and someone
    /// who wants the fuzz on a plastic should not have to switch type to reach
    /// it.
    show_all: bool,
    /// In-progress rename, held while the field has focus so that each
    /// keystroke does not become an undo step.
    renaming: Option<(MaterialKey, String)>,
    /// The rendered preview, and the material it is of.
    ///
    /// Keyed by content rather than by key, so it re-renders when a slider
    /// moves and does not when the selection moves between two parts sharing
    /// one material.
    preview: Option<(u64, egui::TextureHandle)>,
    preview_job: Option<std::thread::JoinHandle<Option<(u64, egui::ColorImage)>>>,
    device: Option<std::sync::Arc<tequila_render_cpu::Device>>,
    /// What the last save to the library said, shown under the button.
    saved: Option<String>,
    /// The library's categories, so a save can file into one that exists.
    /// Set by the app from the browser before the panel draws.
    pub library_categories: Vec<String>,
    /// A save to the library being filled in: the material, and the name
    /// and category it will be saved under.
    save_draft: Option<(MaterialKey, String, String)>,
}

impl MaterialTab {
    /// What the last save to the library said, taken once.
    pub fn take_saved(&mut self) -> Option<String> {
        self.saved.take()
    }

    /// Render (or reuse) the preview sphere for `material`.
    ///
    /// Render on one background worker and discard stale slider results, so
    /// editing never waits for a thumbnail to finish tracing.
    fn preview(
        &mut self,
        ui: &mut egui::Ui,
        graph: &tequila_materials::graph::MaterialGraph,
        textures: std::sync::Arc<tequila_render_core::shade::TextureTable>,
    ) {
        const SIZE: usize = 72;
        let key = crate::library_tab::graph_key(graph);
        if self
            .preview_job
            .as_ref()
            .is_some_and(|job| job.is_finished())
        {
            if let Some(Ok(Some((finished_key, image)))) =
                self.preview_job.take().map(|job| job.join())
            {
                if finished_key == key {
                    self.preview = Some((
                        key,
                        ui.ctx().load_texture(
                            "material-preview",
                            image,
                            egui::TextureOptions::LINEAR,
                        ),
                    ));
                }
            }
        }
        let size = SIZE * crate::thumbnail::density(ui.ctx());
        if self.preview_job.is_none() && self.preview.as_ref().map(|(k, _)| *k) != Some(key) {
            if self.device.is_none() {
                self.device = tequila_render_cpu::Device::shared().ok();
            }
            if let Some(device) = &self.device {
                let device = device.clone();
                let graph = graph.clone();
                let repaint = ui.ctx().clone();
                self.preview_job = std::thread::Builder::new()
                    .name("material-preview".into())
                    .spawn(move || {
                        let settings = tequila_render_cpu::PreviewSettings {
                            size,
                            // Low, deliberately. This runs inside the frame, so it has
                            // to fit inside one; a noisier sphere that keeps up with
                            // the slider is worth more than a clean one that lags.
                            samples: 24,
                            shape: tequila_render_cpu::PreviewShape::Sphere,
                            radius: tequila_render_cpu::PREVIEW_RADIUS,
                        };
                        let environment =
                            std::sync::Arc::new(tequila_scene::demo::preview_environment());
                        if let Ok(thumbnail) = {
                            static POOL: std::sync::OnceLock<rayon::ThreadPool> =
                                std::sync::OnceLock::new();
                            let pool = POOL.get_or_init(|| {
                                rayon::ThreadPoolBuilder::new()
                                    .num_threads(1)
                                    .build()
                                    .expect("preview worker")
                            });
                            pool.install(|| {
                                tequila_render_cpu::render_thumbnail_textured(
                                    &device,
                                    &graph,
                                    textures,
                                    environment,
                                    settings,
                                )
                            })
                        } {
                            // The same development the library's tiles get, so a
                            // material looks the same on the shelf and in the panel.
                            let image = crate::thumbnail::develop(&thumbnail);
                            repaint.request_repaint();
                            return Some((key, image));
                        }
                        repaint.request_repaint();
                        None
                    })
                    .ok();
            }
        }

        match &self.preview {
            Some((_, texture)) => {
                ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(72.0, 72.0)));
            }
            None => {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(72.0, 72.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 6.0, crate::theme::p().control);
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        let Some(node) = project.selection.active() else {
            empty_state(
                ui,
                crate::icons::Icon::Material,
                "Pick a part to dress",
                "Click a part in the viewport or the Scene tree, and its material opens here.",
            );
            return;
        };
        // A group has no material of its own: it shows its first part's,
        // and the part stands in for it below.
        let material_of = |id: NodeId| project.document().tree.get(id).and_then(|n| n.material);
        let shown = match material_of(node) {
            Some(key) => Some((node, key)),
            None => tequila_scene::apply::parts_of(project.document(), [node]).into_iter().find_map(|part| material_of(part).map(|key| (part, key))),
        };
        let Some((node, key)) = shown else {
            empty_state(
                ui,
                crate::icons::Icon::Library,
                "This part has no material yet",
                "Drag one from the Library onto the part, or double-click a library \
                 material with the part selected.",
            );
            return;
        };
        let Some(definition) = project.document().materials.get(key) else {
            return;
        };

        let name = definition.name.clone();
        let material_type = definition.material_type;
        let material = definition.material().clone();
        // Which parameters the graph drives, and the file behind each, so the
        // panel can name it rather than saying only that something does.
        let connected: Vec<(Param, String)> = Param::ALL
            .iter()
            .copied()
            .filter_map(|p| {
                let driver = definition.graph.driver_of(p)?;
                let name = texture_behind(&definition.graph, driver)
                    .and_then(|id| project.document().textures.get(id))
                    .map(|source| source.display_name())
                    .unwrap_or_else(|| "graph".to_string());
                Some((p, name))
            })
            .collect();
        let users = project.document().tree.users_of_material(key).len();
        // Cloned rather than borrowed: the loop below issues commands against
        // `project`, and holding a reference into its materials across that
        // would not compile — and should not, since a command may replace the
        // very graph being read.
        let definition_graph = definition.graph.clone();

        // ---- identity ---------------------------------------------------
        let mut command: Option<Command> = None;
        let mut commit = false;
        let mut pending_texture: Option<Param> = None;
        let mut pending_volume: Option<Param> = None;
        let mut pending_procedural: Option<(Param, tequila_texture::procedural::Procedural)> =
            None;
        let mut save_to_library: Option<(String, String)> = None;
        // A downloaded PBR pack, picked as a folder or as a ZIP / map file.
        let mut pending_set: Option<bool> = None;
        // Another of the project's materials, for the selected parts.
        let mut pending_material: Option<tequila_scene::tree::MaterialKey> = None;

        // A preview of *this* material, rendered by the renderer that will
        // render it. The library's thumbnails answer "what is this material";
        // this answers the question the panel is for, which is "what did that
        // slider just do" — and answers it on a sphere under known lighting,
        // where a change is visible, rather than on whatever corner of the
        // product happens to be in frame.
        crate::theme::card(ui, |ui| ui.horizontal(|ui| {
            // The project's own pictures, so a textured or labelled
            // material previews as itself rather than as its fallbacks.
            let textures = project.scene().textures.clone();
            self.preview(ui, &definition_graph, textures);
            ui.vertical(|ui| {
                ui.add_space(2.0);
                // The name doubles as a picker for every material already in
                // the project: a texture pack with several sets adds them
                // all, and this is where the rest are found.
                egui::ComboBox::from_id_salt("project-material")
                    .selected_text(egui::RichText::new(&name).font(crate::theme::strong(13.0)))
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        ui.label(
                            egui::RichText::new("Project material")
                                .small()
                                .color(crate::theme::dim()),
                        );
                        for (other, definition) in project.document().materials.iter() {
                            if ui.selectable_label(other == key, &definition.name).clicked() && other != key {
                                pending_material = Some(other);
                            }
                        }
                    })
                    .response
                    .on_hover_text("Put another material from this project on the selected parts");
                ui.label(
                    egui::RichText::new(material_type.label())
                        .small()
                        .color(crate::theme::dim()),
                );
                ui.add_space(2.0);
                if ui
                    .add(egui::Button::new("Edit graph…").min_size(egui::vec2(ui.available_width(), 0.0)))
                    .on_hover_text("Build this material from nodes: textures, patterns, colour ramps")
                    .clicked()
                {
                    self.open_graph = Some(key);
                }
                ui.horizontal(|ui| {
                // A draft for this material, started when the menu first
                // opens on it: the name as it is, the category guessed.
                if self.save_draft.as_ref().is_none_or(|(k, _, _)| *k != key) {
                    self.save_draft = Some((key, name.clone(), suggest_category(&name, material_type)));
                }
                let categories = &self.library_categories;
                let draft = self.save_draft.as_mut().expect("just made");
                ui.menu_button("Save to library…", |ui| {
                    ui.set_min_width(240.0);
                    ui.label(
                        egui::RichText::new(
                            "A copy for your own library, patterns and image maps and \
                             all, to reuse in any project. The maps are copied into the \
                             library, so it keeps working when the pack's folder is \
                             tidied away. Labels stay with this project.",
                        )
                        .small()
                        .color(crate::theme::dim()),
                    );
                    egui::Grid::new("save-to-library").num_columns(2).show(ui, |ui| {
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut draft.1).desired_width(170.0));
                        ui.end_row();
                        ui.label("Category");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut draft.2).desired_width(130.0));
                            egui::ComboBox::from_id_salt("save-category")
                                .selected_text("")
                                .width(24.0)
                                .show_ui(ui, |ui| {
                                    for category in categories {
                                        if ui.selectable_label(&draft.2 == category, category).clicked() {
                                            draft.2 = category.clone();
                                        }
                                    }
                                });
                        });
                        ui.end_row();
                    });
                    let ready = !draft.1.trim().is_empty() && !draft.2.trim().is_empty();
                    if ui.add_enabled(ready, egui::Button::new("Save")).clicked() {
                        save_to_library = Some((draft.1.trim().to_string(), draft.2.trim().to_string()));
                        ui.close_menu();
                    }
                });
                ui.menu_button("Texture pack…", |ui| {
                    pending_set = crate::texture_pack::menu(ui);
                })
                .response
                .on_hover_text(
                    "Make a material from a downloaded PBR pack (a folder or a ZIP) \
                     and put it on every part using this one",
                );
                });
            });
        }));
        if let Some((name, category)) = save_to_library {
            // Saved as a *copy*, into the user's own directory. It does not
            // become linked to the material in the document: a library entry
            // is a starting point, and editing this part's material later must
            // not silently rewrite the preset every other project pulls from.
            // Patterns and image maps go with it; labels stay with the project.
            self.save_draft = None;
            let entry = tequila_format::library::LibraryEntry::from_graph_with_pictures(
                name.clone(),
                category.clone(),
                material_type,
                &definition_graph,
                &project.document().textures,
                tequila_format::library::Source::User,
            );
            self.saved = Some(
                match tequila_format::library::save_entry(
                    &crate::library_tab::user_library_path(),
                    &entry,
                ) {
                    Ok(_) => format!("Saved `{name}` to your library, under {category}"),
                    Err(error) => error.to_string(),
                },
            );
        }
        ui.add_space(6.0);
        crate::theme::section(ui, "Material");

        ui.horizontal(|ui| {
            ui.label("Name");
            let mut editing = match &self.renaming {
                Some((k, text)) if *k == key => text.clone(),
                _ => name.clone(),
            };
            let field = ui.add(egui::TextEdit::singleline(&mut editing).desired_width(160.0));
            if field.changed() {
                self.renaming = Some((key, editing.clone()));
            }
            // One undo step per rename, not one per keystroke.
            if field.lost_focus() {
                if let Some((k, text)) = self.renaming.take() {
                    if k == key && text != name && !text.trim().is_empty() {
                        command = Some(Command::RenameMaterial {
                            material: k,
                            name: text,
                        });
                        commit = true;
                    }
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("Type");
            egui::ComboBox::from_id_salt("material_type")
                .selected_text(material_type.label())
                .width(160.0)
                .show_ui(ui, |ui| {
                    for candidate in MaterialType::ALL {
                        let mut chosen = material_type;
                        if ui
                            .selectable_value(&mut chosen, candidate, candidate.label())
                            .clicked()
                            && candidate != material_type
                        {
                            command = Some(Command::SetMaterialType {
                                material: key,
                                material_type: candidate,
                            });
                            commit = true;
                        }
                    }
                });
        });

        // A texture pack's maps share one mapping: its size where anyone
        // looks for it, not only under each map's own Mapping section.
        if let Some((mapping_node, mapping)) = shared_mapping(&definition_graph).filter(|(_, m)| !m.real_world_units) {
            ui.horizontal(|ui| {
                ui.label("Texture repeats");
                let mut across = mapping.scale.x;
                let mut down = mapping.scale.y;
                let a = ui.add(egui::DragValue::new(&mut across).speed(0.05).range(0.01..=1000.0).max_decimals(2))
                    .on_hover_text("How many times the texture repeats across the part's UVs.");
                ui.label("×");
                let b = ui.add(egui::DragValue::new(&mut down).speed(0.05).range(0.01..=1000.0).max_decimals(2))
                    .on_hover_text("And down them. A texture pack starts with the repeats that keep it in proportion on the part.");
                if a.changed() || b.changed() {
                    let mut graph = definition_graph.clone();
                    if let Some(tequila_materials::graph::NodeKind::Mapping2d { mapping }) =
                        graph.node_mut(mapping_node).map(|n| &mut n.kind)
                    {
                        mapping.scale.x = across;
                        mapping.scale.y = down;
                    }
                    command = Some(Command::SetGraph { material: key, graph: Box::new(graph) });
                }
                if a.drag_stopped() || a.lost_focus() || b.drag_stopped() || b.lost_focus() {
                    commit = true;
                }
            });
        }
        if let Some((mapping_node, mapping)) = shared_mapping(&definition_graph).filter(|(_, m)| m.real_world_units) {
            ui.horizontal(|ui| {
                ui.label("Texture size");
                let mut centimetres = mapping.scale.x * 100.0;
                let response = ui
                    .add(
                        egui::DragValue::new(&mut centimetres)
                            .speed(0.1)
                            .range(0.05..=100_000.0)
                            .max_decimals(2)
                            .suffix(" cm"),
                    )
                    .on_hover_text(
                        "How wide one repeat of the texture is on the part. A texture \
                         pack starts as wide as the parts it was put on, the way its \
                         preview lies across its ball; a real brick wall or a floor \
                         wants its real size.",
                    );
                if response.changed() {
                    let metres = centimetres / 100.0;
                    let mut graph = definition_graph.clone();
                    if let Some(tequila_materials::graph::NodeKind::Mapping2d { mapping }) =
                        graph.node_mut(mapping_node).map(|n| &mut n.kind)
                    {
                        mapping.scale.x = metres;
                        mapping.scale.y = metres;
                    }
                    command = Some(Command::SetGraph { material: key, graph: Box::new(graph) });
                }
                if response.drag_stopped() || response.lost_focus() {
                    commit = true;
                }
            });
        }

        Self::linking_badge(ui, node, users, &mut command, &mut commit);
        if !definition_graph.decals.is_empty() {
            ui.small("Labels appear in the viewport and are saved with the project.");
        }
        ui.separator();
        if let Some(index) = crate::decal_tab::show(ui, project, key, self.placing_label) {
            self.place_label_request = Some((key, index));
        }
        ui.separator();

        // ---- parameters -------------------------------------------------
        let exposed = material_type.exposed();
        let shown: Vec<Param> = if self.show_all {
            Param::ALL.to_vec()
        } else {
            exposed.clone()
        };

        if shown.is_empty() {
            ui.label(
                egui::RichText::new("This type has no adjustable parameters.")
                    .small()
                    .weak(),
            );
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for param in shown {
                // In "show all", mark the ones this type would normally hide,
                // so the panel still communicates what the preset is *for*.
                let hidden = self.show_all && !exposed.contains(&param);
                // The tangent is a direction a graph draws - a brushing
                // pattern's own - not a value or a picture: shown where the
                // graph drives it, and not otherwise.
                if param == Param::Tangent && !connected.iter().any(|(p, _)| *p == param) {
                    continue;
                }
                // §4.2: where the graph drives a parameter, the panel says so
                // and offers a way to reach the node, rather than showing a
                // slider whose value the graph would immediately overwrite.
                if let Some((_, driver)) = connected.iter().find(|(p, _)| *p == param) {
                    if connected_row(ui, param, driver, hidden) {
                        command = Some(Command::SetParameterTexture {
                            material: key,
                            param,
                            source: None,
                        });
                        commit = true;
                    }
                    // §5.3's shared mapping controls, on the map they belong
                    // to. These existed in the graph and were evaluated on
                    // every lookup, with nothing anywhere able to change them:
                    // a texture could be attached and then never scaled,
                    // rotated, tiled or projected any way but UV — which on
                    // CAD geometry, where there are frequently no UVs at all,
                    // means the texture could not be used.
                    // A procedural's own controls, where one is what drives
                    // the parameter. Without these it would be four fixed
                    // patterns rather than four adjustable ones, which is most
                    // of the way to not having them.
                    if let Some((current, seed)) =
                        tequila_scene::command::procedural_of(&definition_graph, param)
                    {
                        if let Some(next) = procedural_controls(ui, param, current) {
                            command = Some(Command::SetParameterProcedural {
                                material: key,
                                param,
                                procedural: Some(Box::new(next)),
                                seed,
                            });
                            commit = true;
                        }
                        // A colour parameter reads its procedural through a
                        // ramp, and the ramp is the half that decides whether
                        // the result is wood or a grey field.
                        if let Some(ramp) =
                            tequila_scene::command::gradient_of(&definition_graph, param)
                        {
                            if let Some(next) = gradient_controls(ui, param, &ramp) {
                                command = Some(Command::SetParameterGradient {
                                    material: key,
                                    param,
                                    gradient: Box::new(next),
                                });
                                commit = true;
                            }
                        }
                    }
                    // How deep a pattern or height map stands, through its
                    // Bump node, in millimetres: the control a bump needs most
                    // and the one that was only in the graph editor.
                    if let Some(depth) = tequila_scene::command::bump_depth_of(&definition_graph, param) {
                        let mut millimetres = depth * 1000.0;
                        let response = crate::field::column(ui, ui.available_width() - 33.0, |ui| {
                            let slider = ui.add(
                                crate::field::Slider::new(&mut millimetres, 0.001..=10.0)
                                    .logarithmic(true)
                                    .suffix(" mm")
                                    .text("Depth"),
                            );
                            slider.on_hover_text("How far the pattern's highest point stands above its lowest.")
                        });
                        if response.inner.changed() {
                            let mut graph = definition_graph.clone();
                            if tequila_scene::command::set_bump_depth(&mut graph, param, millimetres / 1000.0) {
                                command = Some(Command::SetGraph { material: key, graph: Box::new(graph) });
                                commit = true;
                            }
                        }
                    }
                    if let Some(current) =
                        tequila_scene::command::mapping_of(&definition_graph, param)
                    {
                        if let Some(next) = mapping_controls(ui, param, current) {
                            command = Some(Command::SetTextureMapping {
                                material: key,
                                param,
                                mapping: Box::new(next),
                            });
                            commit = true;
                        }
                    }
                    continue;
                }
                ui.horizontal(|ui| {
                    // Every row's control the same width, whatever its name,
                    // with the texture button in a column at its right: the
                    // fields line up down the panel. One child for the whole
                    // control, so the hover covers it all, not just whichever
                    // widget the pointer happens to be over.
                    let row = crate::field::column(ui, ui.available_width() - 33.0, |ui| {
                        parameter_row(ui, param, &material, hidden)
                    });
                    if let Some(reason) = param.unimplemented() {
                        row.response.on_hover_text(reason);
                    }
                    if let Some((edit, ended)) = row.inner {
                        command = Some(Command::SetMaterialParameter {
                            material: key,
                            parameter: edit,
                        });
                        commit |= ended;
                    }
                    // §4.2: the toggle that turns a literal into a texture.
                    // Only where a texture would mean something — a metal
                    // preset and a flag have nothing to map.
                    if takes_a_texture(param) {
                        // A menu rather than a button, because there are now
                        // two ways to drive a parameter and a file picker is
                        // only one of them. Procedurals sit beside images
                        // rather than behind the graph editor: gating them
                        // there would have meant shipping four nodes nobody
                        // could reach, which is the trap this codebase has
                        // fallen into twice.
                        // An icon, not the word "tex": it sits on every row,
                        // and a column of the same three letters is noise.
                        let menu = egui::menu::menu_custom_button(
                            ui,
                            egui::Button::new("").frame(false).min_size(egui::vec2(26.0, 26.0)),
                            |ui| {
                            // Density varies through the part's inside, where
                            // only a volume reaches: an image or a pattern is
                            // a surface and would mean nothing there.
                            if param == Param::VolumeDensity {
                                if ui
                                    .button("OpenVDB volume…")
                                    .on_hover_text(
                                        "Vary the density through the part from a .vdb file — smoke, \
                                         cloud, a swirl in a liquid. The grid is fitted to the \
                                         part's bounding box.",
                                    )
                                    .clicked()
                                {
                                    pending_volume = Some(param);
                                    ui.close_menu();
                                }
                                return;
                            }
                            if ui
                                .button("Image…")
                                .on_hover_text("Drive this parameter from a file.")
                                .clicked()
                            {
                                pending_texture = Some(param);
                                ui.close_menu();
                            }
                            ui.separator();
                            for candidate in tequila_texture::procedural::Procedural::catalog() {
                                if ui
                                    .button(candidate.label())
                                    .on_hover_text(procedural_hint(&candidate))
                                    .clicked()
                                {
                                    pending_procedural = Some((param, candidate));
                                    ui.close_menu();
                                }
                            }
                            },
                        );
                        crate::icons::paint_button(ui, menu.response.rect, &menu.response, crate::icons::Icon::Texture, crate::icons::Tone::Quiet, true);
                        menu.response.on_hover_text(if param == Param::VolumeDensity {
                            "Drive this from an OpenVDB volume"
                        } else {
                            "Drive this from an image or a pattern"
                        });
                    }
                });
            }
        });

        ui.separator();
        ui.add(crate::field::Checkbox::new(&mut self.show_all, "Show every parameter"))
            .on_hover_text(
                "The type normally shows only the controls that matter for it. \
                 The rest are still there.",
            );

        // Picking a file is a blocking dialog, so it happens after the panel
        // has finished drawing rather than in the middle of a layout pass.
        if let Some(param) = pending_texture {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(
                    "Images",
                    &["png", "jpg", "jpeg", "psd", "exr", "hdr", "tif", "tiff"],
                )
                .set_title(format!("Image for {}", param.label()))
                .pick_file()
            {
                command = Some(Command::SetParameterTexture {
                    material: key,
                    param,
                    source: Some(TextureSource::new(path.clone(), role_for(param, &path))),
                });
                commit = true;
            }
        }

        if let Some(param) = pending_volume {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("OpenVDB", &["vdb"])
                .set_title(format!("Volume for {}", param.label()))
                .pick_file()
            {
                command = Some(Command::SetParameterTexture {
                    material: key,
                    param,
                    source: Some(TextureSource::new(path, tequila_texture::texture::MapRole::Volume)),
                });
                commit = true;
            }
        }

        if let Some((param, procedural)) = pending_procedural {
            command = Some(Command::SetParameterProcedural {
                material: key,
                param,
                procedural: Some(Box::new(procedural)),
                // Seeded from the material, so two materials differ and the
                // same material is the same every time it is opened. §6 asks
                // for seeding so duplicated parts differ; a random seed here
                // would instead make the *file* differ on every save.
                seed: key.0 as u32,
            });
            commit = true;
        }

        if let Some(folder) = pending_set {
            if let Some(source) = crate::texture_pack::pick(folder) {
                // Every part using this material, as its other controls do.
                let mut users = project.document().tree.users_of_material(key);
                if users.is_empty() {
                    users = vec![node];
                }
                self.saved = Some(match crate::texture_pack::load(project, &source, users) {
                    Ok(loaded) => crate::texture_pack::status(&loaded),
                    Err(error) => error,
                });
            }
        }
        if let Some(material) = pending_material {
            // On the parts: a selected group's own material is never drawn.
            let nodes = tequila_scene::apply::parts_of(project.document(), project.selection.nodes());
            command = Some(Command::AssignMaterial { nodes, material });
            commit = true;
        }

        if let Some(command) = command {
            let _ = project.apply(command);
        }
        // A slider drag is one undo step; releasing it ends the step. Without
        // this, dragging a roughness slider across the panel would leave two
        // hundred entries in the history.
        if commit {
            project.end_interaction();
        }
    }

    /// §1.4, stated rather than implied.
    fn linking_badge(
        ui: &mut egui::Ui,
        node: NodeId,
        users: usize,
        command: &mut Option<Command>,
        commit: &mut bool,
    ) {
        ui.horizontal(|ui| {
            let text = match users {
                0 => "Used by nothing".to_string(),
                1 => "Used by 1 part".to_string(),
                n => format!("Shared by {n} parts"),
            };
            let badge = if users > 1 {
                egui::RichText::new(text).strong()
            } else {
                egui::RichText::new(text).weak()
            };
            ui.label(badge);

            if users > 1 {
                if ui
                    .button("Unlink")
                    .on_hover_text(
                        "Give this part its own copy. Edits will stop affecting the others.",
                    )
                    .clicked()
                {
                    *command = Some(Command::UnlinkMaterial { node });
                    *commit = true;
                }
            }
        });
    }
}

/// One control, chosen by what the parameter says it is.
///
/// Returns the edit and whether the interaction ended, or `None` when nothing
/// changed this frame.
fn parameter_row(
    ui: &mut egui::Ui,
    param: Param,
    material: &tequila_materials::openpbr::Material,
    hidden: bool,
) -> Option<(MaterialParameter, bool)> {
    // A parameter the renderer does not implement is labelled as such rather
    // than left looking live. The control stays usable — the value is stored,
    // saved and will matter the day the feature lands — but nobody should have
    // to discover by experiment that moving it changes no pixel.
    let pending = param.unimplemented();
    let label = match (hidden, pending) {
        (_, Some(_)) => egui::RichText::new(format!("{} · not yet", param.label()))
            .weak()
            .italics(),
        (true, None) => egui::RichText::new(param.label()).weak(),
        (false, None) => egui::RichText::new(param.label()),
    };

    // A slot holds a texture and nothing else, so there is no value to read
    // and no control to draw — but there does have to be a *label*, or the
    // button beside it is a button with no name on it. This was the second
    // half of why bump mapping was unreachable: the row bailed out here before
    // drawing anything at all.
    if param.ui() == ParamUi::Slot {
        // A field's outline with nothing in it: a slot waiting for a map.
        let p = crate::theme::p();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
        ui.painter().rect_stroke(rect.shrink(0.5), 5.0, egui::Stroke::new(1.0_f32, p.line_strong));
        let name = egui::WidgetText::from(label).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (rect.width() - 80.0).max(0.0),
            egui::FontId::proportional(crate::theme::BODY - 0.5),
        );
        ui.painter().galley(egui::pos2(rect.left() + 9.0, rect.center().y - name.size().y * 0.5), name, p.text_dim);
        ui.painter().text(
            egui::pos2(rect.right() - 9.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            "no map",
            egui::FontId::proportional(crate::theme::BODY - 1.5),
            p.text_faint,
        );
        return None;
    }

    let value = param.read(material)?;

    let mut edited: Option<(ParamValue, bool)> = None;

    match (param.ui(), value) {
        (ParamUi::Scalar, ParamValue::Scalar(current)) => {
            let range = param.range()?;
            let mut next = current;
            let mut slider = crate::field::Slider::new(&mut next, range.low..=range.high).text(label);
            if range.logarithmic {
                slider = slider.logarithmic(true);
            }
            if param.is_whole_number() {
                slider = slider.step_by(1.0).fixed_decimals(0);
            }
            let response = ui.add(slider);
            if response.changed() {
                edited = Some((ParamValue::Scalar(next), false));
            }
            if response.drag_stopped() || response.lost_focus() {
                edited = Some((edited.map_or(ParamValue::Scalar(next), |e| e.0), true));
            }
        }

        (ParamUi::Color, ParamValue::Color(current)) => {
            let mut picked = current;
            let response = crate::color_picker::field(ui, label, &mut picked);
            if response.changed() {
                edited = Some((ParamValue::Color(picked), false));
            }
            // The picker is a popup; closing it ends the interaction.
            if response.clicked_elsewhere() || response.lost_focus() {
                if let Some((v, _)) = edited {
                    edited = Some((v, true));
                }
            }
        }

        // Three distances, shown as three numbers. A colour swatch here would
        // invite picking "a nice red" for a mean free path.
        (ParamUi::Vector, ParamValue::Color(current)) => {
            let range = param.range()?;
            ui.horizontal(|ui| {
                ui.label(label);
                let mut v = [current.r, current.g, current.b];
                let mut changed = false;
                let mut ended = false;
                for channel in &mut v {
                    // A hundredth of the value a pixel: a subsurface radius of
                    // 4 mm and a scatter of 30 both move in useful steps,
                    // where a fixed 0.01 tripled the one and crawled the other.
                    let speed = (channel.abs() * 0.01).max(0.0001);
                    let field = ui.add(
                        egui::DragValue::new(channel)
                            .speed(speed)
                            .range(range.low..=range.high),
                    );
                    changed |= field.changed();
                    ended |= field.drag_stopped() || field.lost_focus();
                }
                if changed || ended {
                    edited = Some((
                        ParamValue::Color(Spectrum {
                            r: v[0],
                            g: v[1],
                            b: v[2],
                        }),
                        ended,
                    ));
                }
            });
        }

        (ParamUi::Metal, ParamValue::Metal(current)) => {
            ui.horizontal(|ui| {
                ui.label(label);
                let selected = metals::ALL
                    .iter()
                    .find(|(_, ior)| *ior == current)
                    .map(|(name, _)| *name)
                    .unwrap_or("Custom");
                egui::ComboBox::from_id_salt(param.key())
                    .selected_text(selected)
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        for (name, ior) in metals::ALL {
                            if ui.selectable_label(ior == current, name).clicked() && ior != current
                            {
                                edited = Some((ParamValue::Metal(ior), true));
                            }
                        }
                    });
            });
        }

        (ParamUi::Flag, ParamValue::Flag(current)) => {
            let mut next = current;
            if ui.add(crate::field::Checkbox::new(&mut next, label)).changed() {
                edited = Some((ParamValue::Flag(next), true));
            }
        }

        (ParamUi::Slot, _) => {
            ui.add_enabled(
                false,
                egui::Button::new(egui::RichText::new(format!("{} — texture", param.label()))),
            )
            .on_disabled_hover_text("Texture slots arrive with the material graph.");
        }

        // The reflection table and the presentation table disagreed. A test in
        // `tequila-materials` makes this unreachable; the arm exists so that
        // a future parameter cannot make the panel silently skip a control.
        (ui_kind, stored) => {
            ui.label(
                egui::RichText::new(format!(
                    "{}: presented as {ui_kind:?} but stored as {stored:?}",
                    param.label()
                ))
                .color(crate::theme::p().busy)
                .small(),
            );
        }
    }

    edited.map(|(value, ended)| (MaterialParameter::new(param, value), ended))
}

#[cfg(test)]
mod save_category_tests {
    use super::*;

    /// A downloaded pack's name says what it is; a named type files under
    /// itself; a pack whose name says nothing lands in Other, never in a
    /// category called Generic.
    #[test]
    fn a_save_starts_in_the_category_its_name_suggests() {
        let generic = MaterialType::Generic;
        assert_eq!(suggest_category("Leather037", generic), "Fabric");
        assert_eq!(suggest_category("brown_planks_09", generic), "Wood");
        assert_eq!(suggest_category("Marble012", generic), "Stone");
        assert_eq!(suggest_category("Textile Weave 3", generic), "Fabric");
        assert_eq!(suggest_category("RustedIron02", generic), "Metal");
        assert_eq!(suggest_category("Ground054", generic), "Other");
        assert_eq!(suggest_category("Bracket blue", MaterialType::Plastic), "Plastic");
    }
}

#[cfg(test)]
mod texture_slot_tests {
    use super::*;

    /// Every parameter that could be driven by an image offers the control to
    /// do it — and the one that exists *only* to hold an image most of all.
    ///
    /// It did not. `ParamUi::Slot` is documented as "a texture slot fed by the
    /// material graph; it has no value to show", `Param::Bump` is the one
    /// parameter that has it, and the test for whether a parameter takes a
    /// texture excluded slots. Bump and normal mapping were implemented,
    /// tested at the renderer level, listed as done in the roadmap, and
    /// impossible to reach from the interface.
    #[test]
    fn the_bump_slot_takes_a_texture() {
        assert_eq!(Param::Bump.ui(), ParamUi::Slot);
        assert!(
            takes_a_texture(Param::Bump),
            "the one parameter that is only ever a texture cannot be given one"
        );
    }

    /// Every slot, not just the one that happens to exist today.
    #[test]
    fn every_slot_takes_a_texture() {
        for param in Param::ALL {
            if param.ui() == ParamUi::Slot {
                assert!(
                    takes_a_texture(param),
                    "`{}` is a slot with no way to fill it",
                    param.key()
                );
            }
        }
    }

    /// The genuine exclusions. A measured index of refraction is chosen
    /// from a table, a boolean has nothing to vary across a surface, and a
    /// property of the whole piece — which medium wins an overlap — has one
    /// answer per part, not one per point.
    #[test]
    fn metals_and_flags_do_not() {
        for param in Param::ALL {
            if param.is_per_piece() {
                assert!(!takes_a_texture(param), "`{}` offers a texture across one piece", param.key());
                continue;
            }
            match param.ui() {
                ParamUi::Metal | ParamUi::Flag => assert!(
                    !takes_a_texture(param),
                    "`{}` offers a texture it cannot use",
                    param.key()
                ),
                _ => assert!(takes_a_texture(param), "`{}` takes none", param.key()),
            }
        }
    }

    /// Colour maps decode through sRGB and everything else stays raw. §5.2:
    /// a roughness map read as a colour has its midtones crushed, and the
    /// person who picked the file has no reason to know that.
    #[test]
    fn the_role_follows_the_parameter() {
        let path = std::path::Path::new("surface.png");
        assert_eq!(role_for(Param::BaseColor, path), MapRole::Color);
        assert_eq!(role_for(Param::SpecularRoughness, path), MapRole::Data);
        assert_eq!(
            MapRole::Color.default_color_space(),
            tequila_texture::ColorSpace::Srgb
        );
        assert_eq!(
            MapRole::Data.default_color_space(),
            tequila_texture::ColorSpace::Raw
        );
    }

    /// The bump slot is the one place the parameter alone is not enough: it
    /// takes a height field, whose *gradient* is the bump, or a tangent-space
    /// normal map, whose values *are* the normal. Reading one as the other is
    /// not subtle — a normal map read as height renders as a blue sheen with
    /// the detail flattened away — and every texture pipeline in the world
    /// names them, so the name is what is read.
    #[test]
    fn a_normal_map_is_recognised_by_its_name() {
        let normal = [
            "brick_normal.png",
            "brick-normal.exr",
            "brick_nrm.png",
            "brick_norm.tif",
            "brick_n.png",
            "brickNormalMap.png",
            "brick_normal_map.png",
            // Case is not a signal anybody controls.
            "Brick_NORMAL.PNG",
        ];
        for name in normal {
            assert_eq!(
                role_for(Param::Bump, std::path::Path::new(name)),
                MapRole::Normal,
                "`{name}` was not read as a normal map"
            );
        }

        let height = [
            "brick_height.png",
            "brick_bump.png",
            "brick_displacement.exr",
            "brick.png",
            // A colour map that merely mentions the word must not be caught by
            // a looser rule, which is why the suffixes are anchored.
            "north_wall.png",
        ];
        for name in height {
            assert_eq!(
                role_for(Param::Bump, std::path::Path::new(name)),
                MapRole::Height,
                "`{name}` was read as a normal map"
            );
        }
    }

    /// Both bump roles decode raw. A transfer curve applied to a normal map
    /// bends the vector it encodes; applied to a height field it bends the
    /// slope.
    #[test]
    fn neither_bump_role_is_decoded_through_srgb() {
        for role in [MapRole::Normal, MapRole::Height] {
            assert_eq!(
                role.default_color_space(),
                tequila_texture::ColorSpace::Raw,
                "{role:?} would be decoded through a transfer curve"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scene::demo::{demo_document, DemoMaterial};
    use std::cell::RefCell;

    /// A project with a part selected and a material on it.
    fn project() -> Project {
        let document = demo_document(DemoMaterial::Plastic, 0.25);
        let environment = std::sync::Arc::new(tequila_scene::demo::studio_environment());
        let mut project = Project::with_document(document, environment);
        let part = project
            .document()
            .tree
            .iter()
            .find(|n| n.material.is_some())
            .expect("the demo scene has a material on something")
            .id;
        project
            .selection
            .select_many([part], SelectionMode::Replace);
        project
    }

    /// Draw the panel once, headlessly, and hand back the project.
    fn draw(tab: &RefCell<MaterialTab>, project: &RefCell<Project>) {
        egui::__run_test_ui(|ui| {
            tab.borrow_mut().show(ui, &mut project.borrow_mut());
        });
    }

    /// The panel builds without panicking for **every** type in the catalog,
    /// in both the exposed and the show-everything modes.
    ///
    /// This is the cheap test that would have caught a presentation table
    /// disagreeing with the storage table, a parameter with no range where a
    /// slider needs one, or a duplicated widget id. Thirty-four types times two
    /// modes is sixty-eight panels nobody would click through by hand.
    /// Priority is a whole number for the whole piece: it steps in ones and
    /// offers no texture, where it showed two decimals and a "tex" menu that
    /// would have painted which medium wins across the surface.
    #[test]
    fn priority_is_a_whole_number_with_no_texture() {
        assert!(Param::MediumPriority.is_whole_number());
        assert!(!takes_a_texture(Param::MediumPriority));
        assert!(takes_a_texture(Param::SpecularRoughness), "roughness can still be textured");
    }

    #[test]
    fn every_catalog_type_renders_a_panel() {
        for material_type in MaterialType::ALL {
            for show_all in [false, true] {
                let project = RefCell::new(project());
                let key = project
                    .borrow()
                    .document()
                    .materials
                    .keys()
                    .next()
                    .expect("a material");
                project
                    .borrow_mut()
                    .apply(Command::SetMaterialType {
                        material: key,
                        material_type,
                    })
                    .expect("the type switch applies");

                let tab = RefCell::new(MaterialTab {
                    show_all,
                    ..MaterialTab::default()
                });
                draw(&tab, &project);
            }
        }
    }

    /// With nothing selected the panel says so rather than panicking on the
    /// `None`.
    #[test]
    fn an_empty_selection_is_handled() {
        let project = RefCell::new(project());
        project.borrow_mut().selection.clear();
        let tab = RefCell::new(MaterialTab::default());
        draw(&tab, &project);
    }

    /// Drawing the panel must not, by itself, change the document.
    ///
    /// A control that reports "changed" on the frame it is created turns
    /// merely *looking* at a material into an edit — the file goes modified,
    /// the render restarts, and undo fills with steps the user never took.
    #[test]
    fn drawing_the_panel_does_not_edit_the_material() {
        let project = RefCell::new(project());
        let before = project.borrow().document().materials.clone();
        let generation = project.borrow().generation();

        let tab = RefCell::new(MaterialTab::default());
        for _ in 0..4 {
            draw(&tab, &project);
        }

        assert_eq!(
            project.borrow().document().materials,
            before,
            "drawing the panel modified the material"
        );
        assert_eq!(
            project.borrow().generation(),
            generation,
            "drawing the panel restarted the render"
        );
        assert!(
            !project.borrow().can_undo(),
            "drawing the panel added an undo step"
        );
    }

    /// Switching type through the command bus is undoable as one step, and
    /// undo restores the original type *and* its parameters.
    #[test]
    fn a_type_switch_is_one_undo_step() {
        let mut project = project();
        let key = project
            .document()
            .materials
            .keys()
            .next()
            .expect("a material");
        let before = project
            .document()
            .materials
            .get(key)
            .cloned()
            .expect("a definition");

        project
            .apply(Command::SetMaterialType {
                material: key,
                material_type: MaterialType::Velvet,
            })
            .expect("applies");
        project.end_interaction();
        assert_eq!(
            project.document().materials.get(key).unwrap().material_type,
            MaterialType::Velvet
        );

        project.undo();
        assert_eq!(
            project.document().materials.get(key),
            Some(&before),
            "undo did not restore the material exactly"
        );
    }
}

/// A parameter the graph drives (§4.2). Returns whether the user detached it.
fn connected_row(ui: &mut egui::Ui, param: Param, driver: &str, hidden: bool) -> bool {
    let mut detach = false;
    ui.horizontal(|ui| {
        let label = if hidden {
            egui::RichText::new(param.label()).weak()
        } else {
            egui::RichText::new(param.label())
        };
        ui.label(label);
        ui.label(
            egui::RichText::new(driver)
                .small()
                .color(crate::theme::hue(crate::theme::Hue::Blue)),
        )
        .on_hover_text(
            "This parameter is driven by the material graph. Its value comes from there.",
        );
        if ui
            .small_button("x")
            .on_hover_text("Go back to a plain value.")
            .clicked()
        {
            detach = true;
        }
    });
    detach
}

/// Whether driving this parameter from an image means anything.
///
/// A metal preset is a choice from a measured table and a flag is a yes or no;
/// neither has a per-pixel reading. Offering the toggle anyway would be a
/// control that can only disappoint.
fn takes_a_texture(param: Param) -> bool {
    // A slot is *only* a texture. `ParamUi::Slot` is documented as "a texture
    // slot fed by the material graph; it has no value to show", and Bump is
    // the one parameter that has it — so excluding slots here meant the single
    // parameter that exists to hold a map was the only one that could not be
    // given one. Bump and normal mapping were implemented, tested and
    // unreachable.
    //
    // A metal preset and a flag are the genuine exclusions: a measured index of
    // refraction is chosen from a table and a boolean has nothing to vary
    // across a surface.
    !matches!(param.ui(), ParamUi::Metal | ParamUi::Flag) && !param.is_per_piece()
}

/// The colour ramp a procedural is read through.
///
/// Drawn as the ramp itself with its stops beneath, rather than as a list of
/// numbers: a gradient is a picture, and the thing a person is judging is what
/// it looks like.
pub(crate) fn gradient_controls(
    ui: &mut egui::Ui,
    param: Param,
    current: &tequila_texture::gradient::Gradient,
) -> Option<tequila_texture::gradient::Gradient> {
    use tequila_texture::gradient::{Gradient, Stop};

    let mut next = current.clone();
    let mut changed = false;

    crate::field::Fold::new("Colour ramp")
        .id_salt(("gradient", param.key()))
        .default_open(true)
        .show(ui, |ui| {
            // The ramp, painted. Sampled per pixel of its own width so the
            // preview is the ramp rather than an approximation of it.
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width().min(240.0), 18.0),
                egui::Sense::hover(),
            );
            let painter = ui.painter();
            let steps = rect.width().max(1.0) as usize;
            for i in 0..steps {
                let t = i as f32 / (steps - 1).max(1) as f32;
                let c = next.evaluate(t);
                let srgb = tequila_color::DisplayTransform::Srgb.apply(c);
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(rect.left() + i as f32, rect.top()),
                        egui::vec2(1.0, rect.height()),
                    ),
                    0.0,
                    egui::Color32::from_rgb(
                        (srgb.x * 255.0) as u8,
                        (srgb.y * 255.0) as u8,
                        (srgb.z * 255.0) as u8,
                    ),
                );
            }

            let mut remove = None;
            // Snapshotted before the loop: the rows edit `next`, and iterating
            // its own stops while doing so would borrow it twice.
            let rows: Vec<tequila_texture::gradient::Stop> = next.stops().to_vec();
            for (index, stop) in rows.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut at = stop.at;
                    if ui
                        .add(
                            egui::DragValue::new(&mut at)
                                .speed(0.005)
                                .range(0.0..=1.0)
                                .max_decimals(3),
                        )
                        .changed()
                    {
                        let mut stops = next.stops().to_vec();
                        stops[index].at = at;
                        next = next.with_stops(stops);
                        changed = true;
                    }

                    let mut picked = stop.color;
                    if crate::color_picker::edit(ui, &mut picked).changed() {
                        let mut stops = next.stops().to_vec();
                        stops[index].color = picked;
                        next = next.with_stops(stops);
                        changed = true;
                    }
                    if next.stops().len() > 2 && ui.small_button("x").clicked() {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                if next.remove(index) {
                    changed = true;
                }
            }

            ui.horizontal(|ui| {
                if ui.small_button("+ stop").clicked() {
                    // Halfway along, which is where somebody adding a stop
                    // almost always wants one, and which never lands on an
                    // existing stop in a two-stop ramp.
                    let at = 0.5;
                    let color = next.evaluate(at);
                    next.insert(Stop { at, color });
                    changed = true;
                }
                ui.menu_button("Preset", |ui| {
                    for (name, preset) in Gradient::presets() {
                        if ui.button(name).clicked() {
                            next = preset;
                            changed = true;
                            ui.close_menu();
                        }
                    }
                });
            });
        });

    changed.then_some(next)
}

/// The axis row shared by the nodes that have a direction.
///
/// Labelled by what the axis *does* rather than "Axis": for marble the bands
/// run across it and for wood the grain runs along it, and a person choosing
/// between X, Y and Z is choosing a direction in their model, not filling in a
/// field.
fn axis_row(ui: &mut egui::Ui, label: &str, axis: &mut tequila_texture::procedural::Axis) {
    use tequila_texture::procedural::Axis;
    ui.horizontal(|ui| {
        ui.label(label);
        for candidate in Axis::ALL {
            ui.selectable_value(axis, candidate, candidate.label());
        }
    });
}

/// What a procedural is for, in one line.
fn procedural_hint(p: &tequila_texture::procedural::Procedural) -> &'static str {
    use tequila_texture::procedural::Procedural;
    match p {
        Procedural::Noise { .. } => {
            "Fractal noise. The general-purpose one: fine variation in roughness \
             so a moulded surface is not perfectly uniform."
        }
        Procedural::Cellular { .. } => {
            "Worley cells. Blobs, or the crack network between them \
             — leather, granite, grout."
        }
        Procedural::Scratches { .. } => {
            "Sparse fine scratches. What anything assembled, packed or handled \
             carries, and what stops a polished surface looking moulded from a \
             render farm."
        }
        Procedural::Brushed { .. } => "A directional grain, for brushed and machined metal.",
        Procedural::Marble { .. } => {
            "Bands bent by turbulence, read through a ramp. Marble, and any \
             other veined stone."
        }
        Procedural::Wood { .. } => {
            "Growth rings about an axis, with grain along it. Put it on base \
             colour and take the Wood ramp."
        }
        Procedural::WoodAdvanced { .. } => {
            "Wood with uneven rings, dark latewood, ray flecks, knots and \
             planks. For a board that has to hold up close."
        }
        Procedural::NoiseTexture { .. } => {
            "One layer of soft noise with contrast. For masks: blotches that \
             break up a colour or a coat."
        }
        Procedural::Ridged { .. } => {
            "Sharp crests with the detail gathered on them. Rock, eroded \
             stone, hammered metal."
        }
        Procedural::Granite { .. } => {
            "Crystals at two sizes with flecks. Put it on base colour and give \
             the ramp the stone's colours."
        }
        Procedural::Leather { .. } => {
            "Pebbled grain with cracks between the pebbles. Put it on bump."
        }
        Procedural::Spots { .. } => "Round spots, one per cell, of varying size.",
        Procedural::Camouflage { .. } => {
            "Stacked blobs in flat layers. Give the ramp one colour per layer."
        }
        Procedural::Contour { .. } => {
            "Lines at even steps along an axis, bent by noise: topographic \
             maps and technical looks."
        }
        Procedural::BrushedRadial { .. } => {
            "Brushing in circles about an axis: the face of a machined knob \
             or a watch crown."
        }
        Procedural::Weave { .. } => {
            "Threads crossing over and under: plain, twill or satin. Put it on \
             bump; the seed shifts where the cloth was cut."
        }
        Procedural::Mesh { .. } => {
            "A perforated sheet. Put it on opacity for real holes: speaker \
             grilles, wire mesh."
        }
    }
}

/// The controls for whichever procedural drives a parameter.
pub(crate) fn procedural_controls(
    ui: &mut egui::Ui,
    param: Param,
    current: tequila_texture::procedural::Procedural,
) -> Option<tequila_texture::procedural::Procedural> {
    use tequila_texture::noise::{Basis, Metric};
    use tequila_texture::procedural::{CellOutput, HoleShape, Procedural, WeavePattern};

    let mut next = current;
    crate::field::Fold::new(current.label())
        .id_salt(("procedural", param.key()))
        .default_open(true)
        .show(ui, |ui| match &mut next {
            Procedural::Noise { fractal, frequency } => {
                ui.add(
                    crate::field::Slider::new(frequency, 0.1..=200.0)
                        .text("Scale")
                        .logarithmic(true),
                );
                let mut octaves = fractal.octaves;
                if ui
                    .add(crate::field::Slider::new(&mut octaves, 1..=10).text("Detail"))
                    .on_hover_text(
                        "How many octaves are summed. More is finer detail, not \
                         more contrast — the sum is normalized so adding detail \
                         does not also darken it.",
                    )
                    .changed()
                {
                    fractal.octaves = octaves;
                }
                ui.add(crate::field::Slider::new(&mut fractal.gain, 0.1..=0.9).text("Roughness"));
                ui.add(crate::field::Slider::new(&mut fractal.lacunarity, 1.5..=4.0).text("Lacunarity"));
                ui.horizontal(|ui| {
                    ui.label("Basis");
                    ui.selectable_value(&mut fractal.basis, Basis::Perlin, "Gradient");
                    ui.selectable_value(&mut fractal.basis, Basis::Value, "Value");
                });
            }
            Procedural::Cellular {
                metric,
                output,
                frequency,
            } => {
                ui.add(
                    crate::field::Slider::new(frequency, 0.1..=200.0)
                        .text("Scale")
                        .logarithmic(true),
                );
                ui.horizontal(|ui| {
                    ui.label("Output");
                    ui.selectable_value(output, CellOutput::Distance, "Blobs");
                    ui.selectable_value(output, CellOutput::Edges, "Cracks");
                    ui.selectable_value(output, CellOutput::Id, "Flat cells");
                });
                ui.horizontal(|ui| {
                    ui.label("Shape");
                    ui.selectable_value(metric, Metric::Euclidean, "Round");
                    ui.selectable_value(metric, Metric::Manhattan, "Diamond");
                    ui.selectable_value(metric, Metric::Chebyshev, "Square");
                });
            }
            Procedural::Scratches {
                frequency,
                width,
                density,
                spread,
            } => {
                ui.add(
                    crate::field::Slider::new(frequency, 0.5..=100.0)
                        .text("Scale")
                        .logarithmic(true),
                );
                ui.add(crate::field::Slider::new(width, 0.005..=0.3).text("Width"));
                ui.add(crate::field::Slider::new(density, 0.0..=1.0).text("Density"));
                ui.add(crate::field::Slider::new(spread, 0.0..=0.5).text("Spread"))
                    .on_hover_text(
                        "How far a scratch may turn from the reference \
                         direction. Zero is perfectly parallel, which is a \
                         brushed finish; a quarter turn is random, which is \
                         wear.",
                    );
            }
            Procedural::Brushed { frequency, stretch } => {
                ui.add(
                    crate::field::Slider::new(frequency, 1.0..=400.0)
                        .text("Fineness")
                        .logarithmic(true),
                );
                ui.add(
                    crate::field::Slider::new(stretch, 1.0..=400.0)
                        .text("Grain length")
                        .logarithmic(true),
                );
            }
            Procedural::Marble {
                axis,
                frequency,
                distortion,
                fractal,
            } => {
                axis_row(ui, "Bands across", axis);
                ui.add(
                    crate::field::Slider::new(frequency, 0.1..=40.0)
                        .text("Band spacing")
                        .logarithmic(true),
                );
                ui.add(crate::field::Slider::new(distortion, 0.0..=10.0).text("Vein distortion"))
                    .on_hover_text(
                        "How far the turbulence bends the bands. At zero they are \
                         straight stripes; this is the whole difference between \
                         marble and a barcode.",
                    );
                ui.add(crate::field::Slider::new(&mut fractal.octaves, 1..=8).text("Vein detail"));
                ui.add(crate::field::Slider::new(&mut fractal.gain, 0.1..=0.9).text("Vein roughness"));
            }
            Procedural::WoodAdvanced {
                axis,
                frequency,
                distortion,
                grain,
                fractal,
                ring_variation,
                latewood,
                contrast,
                flecks,
                knots,
                board_width,
            } => {
                axis_row(ui, "Grain along", axis);
                ui.add(crate::field::Slider::new(frequency, 0.1..=60.0).text("Rings").logarithmic(true));
                ui.add(crate::field::Slider::new(distortion, 0.0..=2.0).text("Ring wander"));
                ui.add(crate::field::Slider::new(ring_variation, 0.0..=1.0).text("Uneven years"))
                    .on_hover_text("Good years and bad: how much ring widths vary.");
                ui.add(crate::field::Slider::new(latewood, 0.05..=0.9).text("Latewood"))
                    .on_hover_text("The dark share of each ring.");
                ui.add(crate::field::Slider::new(contrast, 0.0..=1.0).text("Ring contrast"));
                ui.add(crate::field::Slider::new(grain, 0.0..=1.0).text("Grain"));
                ui.add(crate::field::Slider::new(flecks, 0.0..=1.0).text("Ray flecks"))
                    .on_hover_text("Short streaks radiating from the heart. Quarter-sawn oak is full of them.");
                ui.add(crate::field::Slider::new(knots, 0.0..=1.0).text("Knots"));
                ui.add(crate::field::Slider::new(board_width, 0.0..=2.0).text("Plank width"))
                    .on_hover_text("Planks side by side, each from its own log. Zero is one piece.");
                ui.add(crate::field::Slider::new(&mut fractal.octaves, 1..=6).text("Wander detail"));
            }
            Procedural::Wood {
                axis,
                frequency,
                distortion,
                grain,
                fractal,
            } => {
                axis_row(ui, "Grain along", axis);
                ui.add(
                    crate::field::Slider::new(frequency, 0.1..=60.0)
                        .text("Rings")
                        .logarithmic(true),
                );
                ui.add(crate::field::Slider::new(distortion, 0.0..=2.0).text("Ring wander"))
                    .on_hover_text(
                        "Real growth rings are never concentric. At zero this \
                         draws a target painted on the surface.",
                    );
                ui.add(crate::field::Slider::new(grain, 0.0..=1.0).text("Grain"))
                    .on_hover_text(
                        "The fine streaking along the grain. Without it the rings \
                         read as printed on.",
                    );
                ui.add(crate::field::Slider::new(&mut fractal.octaves, 1..=8).text("Ring detail"));
            }
            Procedural::NoiseTexture {
                basis,
                frequency,
                contrast,
                bias,
            } => {
                ui.add(crate::field::Slider::new(frequency, 0.1..=200.0).text("Scale").logarithmic(true));
                ui.add(crate::field::Slider::new(contrast, 0.0..=8.0).text("Contrast"))
                    .on_hover_text("Around the middle grey: above 1 pushes it toward a hard mask.");
                ui.add(crate::field::Slider::new(bias, -1.0..=1.0).text("Bias"));
                ui.horizontal(|ui| {
                    ui.label("Basis");
                    ui.selectable_value(basis, Basis::Perlin, "Gradient");
                    ui.selectable_value(basis, Basis::Value, "Value");
                });
            }
            Procedural::Ridged { fractal, frequency } => {
                ui.add(crate::field::Slider::new(frequency, 0.1..=200.0).text("Scale").logarithmic(true));
                ui.add(crate::field::Slider::new(&mut fractal.octaves, 1..=10).text("Detail"));
                ui.add(crate::field::Slider::new(&mut fractal.gain, 0.1..=0.9).text("Roughness"));
                ui.add(crate::field::Slider::new(&mut fractal.lacunarity, 1.5..=4.0).text("Lacunarity"));
            }
            Procedural::Granite {
                frequency,
                speckle,
                fine,
            } => {
                ui.add(crate::field::Slider::new(frequency, 0.5..=200.0).text("Scale").logarithmic(true));
                ui.add(crate::field::Slider::new(speckle, 0.0..=1.0).text("Flecks"));
                ui.add(crate::field::Slider::new(fine, 0.0..=1.0).text("Fine grain"));
            }
            Procedural::Leather {
                frequency,
                crack_width,
                wrinkle,
            } => {
                ui.add(crate::field::Slider::new(frequency, 1.0..=400.0).text("Pebble size").logarithmic(true))
                    .on_hover_text("Pebbles per metre. Higher is a finer grain.");
                ui.add(crate::field::Slider::new(crack_width, 0.0..=0.5).text("Cracks"));
                ui.add(crate::field::Slider::new(wrinkle, 0.0..=1.0).text("Wrinkle"));
            }
            Procedural::Spots {
                frequency,
                coverage,
                variance,
                softness,
            } => {
                ui.add(crate::field::Slider::new(frequency, 0.5..=200.0).text("Scale").logarithmic(true));
                ui.add(crate::field::Slider::new(coverage, 0.0..=1.0).text("Size"));
                ui.add(crate::field::Slider::new(variance, 0.0..=1.0).text("Size variation"));
                ui.add(crate::field::Slider::new(softness, 0.0..=1.0).text("Softness"));
            }
            Procedural::Camouflage { frequency, layers } => {
                ui.add(crate::field::Slider::new(frequency, 0.1..=50.0).text("Scale").logarithmic(true));
                ui.add(crate::field::Slider::new(layers, 2..=5).text("Layers"));
            }
            Procedural::Contour {
                axis,
                frequency,
                width,
                softness,
                distortion,
            } => {
                axis_row(ui, "Steps along", axis);
                ui.add(crate::field::Slider::new(frequency, 0.5..=200.0).text("Lines").logarithmic(true));
                ui.add(crate::field::Slider::new(width, 0.0..=1.0).text("Line width"));
                ui.add(crate::field::Slider::new(softness, 0.0..=1.0).text("Softness"));
                ui.add(crate::field::Slider::new(distortion, 0.0..=4.0).text("Wander"))
                    .on_hover_text("How far noise bends the lines. Zero is straight.");
            }
            Procedural::BrushedRadial {
                axis,
                frequency,
                stretch,
            } => {
                axis_row(ui, "Around", axis);
                ui.add(crate::field::Slider::new(frequency, 1.0..=1000.0).text("Fineness").logarithmic(true));
                ui.add(crate::field::Slider::new(stretch, 1.0..=400.0).text("Grain length").logarithmic(true));
            }
            Procedural::Weave {
                pattern,
                frequency,
                width,
                variance,
            } => {
                ui.horizontal(|ui| {
                    ui.label("Weave");
                    ui.selectable_value(pattern, WeavePattern::Plain, "Plain");
                    ui.selectable_value(pattern, WeavePattern::Twill, "Twill");
                    ui.selectable_value(pattern, WeavePattern::Satin, "Satin");
                });
                ui.add(crate::field::Slider::new(frequency, 1.0..=1000.0).text("Threads").logarithmic(true))
                    .on_hover_text("Threads per metre.");
                ui.add(crate::field::Slider::new(width, 0.1..=1.0).text("Thread width"));
                ui.add(crate::field::Slider::new(variance, 0.0..=1.0).text("Unevenness"));
            }
            Procedural::Mesh {
                shape,
                frequency,
                size,
                softness,
            } => {
                ui.horizontal(|ui| {
                    ui.label("Holes");
                    ui.selectable_value(shape, HoleShape::Round, "Round");
                    ui.selectable_value(shape, HoleShape::Square, "Square");
                    ui.selectable_value(shape, HoleShape::Hex, "Hex");
                });
                ui.add(crate::field::Slider::new(frequency, 1.0..=500.0).text("Holes per unit").logarithmic(true));
                ui.add(crate::field::Slider::new(size, 0.0..=1.0).text("Hole size"));
                ui.add(crate::field::Slider::new(softness, 0.0..=1.0).text("Softness"));
            }
        });

    (next != current).then_some(next)
}

/// §5.3's mapping controls for one texture.
///
/// Returns the edited mapping when something changed. Folded away by default:
/// a texture usually needs none of this, and eleven controls under every map
/// would bury the parameters they belong to.
pub(crate) fn mapping_controls(
    ui: &mut egui::Ui,
    param: Param,
    current: tequila_texture::projection::Mapping,
) -> Option<tequila_texture::projection::Mapping> {
    use tequila_texture::mip::Wrap;
    use tequila_texture::projection::Projection;

    let mut next = current;
    crate::field::Fold::new("Mapping")
        .id_salt(("mapping", param.key()))
        .default_open(false)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Projection");
                egui::ComboBox::from_id_salt(("projection", param.key()))
                    .selected_text(projection_label(next.projection))
                    .show_ui(ui, |ui| {
                        for candidate in [
                            Projection::Uv,
                            Projection::TriPlanar,
                            Projection::Planar,
                            Projection::Cylindrical,
                            Projection::Spherical,
                        ] {
                            ui.selectable_value(
                                &mut next.projection,
                                candidate,
                                projection_label(candidate),
                            );
                        }
                    });
            });
            if next.projection == Projection::Uv {
                ui.horizontal(|ui| {
                    ui.label("UV set");
                    let mut set = next.uv_set as u32;
                    if ui
                        .add(egui::DragValue::new(&mut set).range(0..=7))
                        .changed()
                    {
                        next.uv_set = set as usize;
                    }
                });
            }
            if next.projection == Projection::TriPlanar {
                ui.add(
                    crate::field::Slider::new(&mut next.blend_sharpness, 1.0..=16.0)
                        .text("Blend sharpness"),
                )
                .on_hover_text(
                    "How tightly the three projections blend across an edge. \
                     Higher is a crisper join on a flat face; lower is a softer \
                     transition around a corner.",
                );
            }

            ui.add(crate::field::Checkbox::new(&mut next.real_world_units, "Real-world size"))
                .on_hover_text(
                    "Read the scale as a size in metres rather than as a \
                     number of repeats, so a texture specified as 50 mm across \
                     stays 50 mm across on a bracket and on a door panel alike.",
                );

            ui.horizontal(|ui| {
                ui.label(if next.real_world_units {
                    "Size"
                } else {
                    "Repeats"
                });
                for value in [&mut next.scale.x, &mut next.scale.y, &mut next.scale.z] {
                    ui.add(
                        egui::DragValue::new(value)
                            .speed(0.01)
                            .range(0.001..=10_000.0),
                    );
                }
            });
            ui.horizontal(|ui| {
                ui.label("Offset");
                ui.add(egui::DragValue::new(&mut next.offset.x).speed(0.005));
                ui.add(egui::DragValue::new(&mut next.offset.y).speed(0.005));
            });
            ui.horizontal(|ui| {
                ui.label("Rotation");
                // In turns, which is what the mapping stores. Degrees on the
                // face of it, because nobody thinks in turns.
                let mut degrees = next.rotation * 360.0;
                if ui
                    .add(
                        egui::DragValue::new(&mut degrees)
                            .speed(1.0)
                            .range(-360.0..=360.0)
                            .suffix("°"),
                    )
                    .changed()
                {
                    next.rotation = degrees / 360.0;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Wrap");
                for (label, wrap) in [
                    ("Repeat", Wrap::Repeat),
                    ("Clamp", Wrap::Clamp),
                    ("Mirror", Wrap::Mirror),
                ] {
                    // One control for both axes. They can differ in the data,
                    // and a person who needs them to differ is a person in the
                    // graph editor; two rows here would cost more than it is
                    // worth to everybody else.
                    let selected = next.wrap_u == wrap && next.wrap_v == wrap;
                    if ui.selectable_label(selected, label).clicked() {
                        next.wrap_u = wrap;
                        next.wrap_v = wrap;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Flip");
                ui.add(crate::field::Checkbox::new(&mut next.flip_u, "U"));
                ui.add(crate::field::Checkbox::new(&mut next.flip_v, "V"));
            });
        });

    (next != current).then_some(next)
}

fn projection_label(projection: tequila_texture::projection::Projection) -> &'static str {
    use tequila_texture::projection::Projection;
    match projection {
        Projection::Uv => "UV",
        Projection::Planar => "Planar",
        Projection::Cylindrical => "Cylindrical",
        Projection::Spherical => "Spherical",
        Projection::TriPlanar => "Tri-planar",
    }
}

/// The map role a parameter implies, which is what decides the colour space the
/// file is decoded with (§5.2).
///
/// Deriving this from the parameter rather than asking is the whole point of
/// §5.2: a roughness map read as a colour has its midtones crushed, and the
/// person who picked the file has no reason to know that, or to be asked.
///
/// The bump slot is the one place the parameter is not enough, because it takes
/// two different things: a height field, whose gradient is the bump, and a
/// tangent-space normal map, whose values *are* the perturbed normal. Reading
/// one as the other is not subtle — a normal map read as height renders as a
/// blue sheen with the detail flattened out. The filename is what distinguishes
/// them in practice, since every texture pipeline in the world writes
/// `_normal`, `_nrm` or `_n`, and getting it wrong is visible immediately and
/// fixed by the role control beside the slot.
fn role_for(param: Param, path: &std::path::Path) -> MapRole {
    match param.ui() {
        ParamUi::Color => MapRole::Color,
        ParamUi::Slot => {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let looks_like_a_normal_map = name.ends_with("_normal")
                || name.ends_with("-normal")
                || name.ends_with("_nrm")
                || name.ends_with("_norm")
                || name.ends_with("_n")
                || name.contains("normalmap")
                || name.contains("normal_map");
            if looks_like_a_normal_map {
                MapRole::Normal
            } else {
                MapRole::Height
            }
        }
        _ => MapRole::Data,
    }
}

/// The library category a material most likely belongs in, for a save to
/// start from. A downloaded pack is a Generic material whose name says what
/// it is — `Leather037`, `brown_planks_09` — so the name is read first; a
/// material of a named type files under that type, as saves always did.
pub fn suggest_category(name: &str, material_type: MaterialType) -> String {
    const WORDS: [(&str, &[&str]); 8] = [
        // Before Fabric: a carbon or aramid weave is a composite, not a cloth.
        ("Composite", &["carbon", "kevlar", "aramid", "fiberglass", "fibreglass", "composite", "forged", "cfrp", "grp"]),
        ("Wood", &["wood", "plank", "oak", "walnut", "pine", "birch", "maple", "bark", "timber", "parquet", "plywood", "veneer", "bamboo"]),
        ("Fabric", &["fabric", "cloth", "leather", "denim", "felt", "velvet", "carpet", "knit", "linen", "wool", "cotton", "canvas", "suede", "wicker", "textile"]),
        ("Stone", &["stone", "marble", "granite", "rock", "concrete", "brick", "tile", "terrazzo", "gravel", "asphalt", "plaster", "cobble", "slate", "sandstone", "cement"]),
        ("Metal", &["metal", "steel", "iron", "rust", "alumin", "brass", "copper", "gold", "silver", "chrome", "bronze", "zinc", "titanium"]),
        ("Plastic", &["plastic", "rubber", "vinyl", "foam"]),
        ("Paint", &["paint", "lacquer", "enamel"]),
        ("Glass", &["glass"]),
    ];
    let lower = name.to_lowercase();
    for (category, words) in WORDS {
        if words.iter().any(|w| lower.contains(w)) {
            return category.to_string();
        }
    }
    match material_type {
        MaterialType::Generic => "Other".to_string(),
        other => other.label().to_string(),
    }
}

/// The one mapping a material's maps share, where it has exactly one: what
/// a texture pack builds.
fn shared_mapping(
    graph: &tequila_materials::graph::MaterialGraph,
) -> Option<(tequila_materials::graph::NodeId, tequila_texture::projection::Mapping)> {
    let mut found = graph.nodes().filter_map(|(id, node)| match &node.kind {
        tequila_materials::graph::NodeKind::Mapping2d { mapping } => Some((id, mapping.clone())),
        _ => None,
    });
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// The texture behind a node, following a chain of pure nodes back if need be.
fn texture_behind(
    graph: &tequila_materials::graph::MaterialGraph,
    node: tequila_materials::graph::NodeId,
) -> Option<tequila_materials::graph::TextureId> {
    use tequila_materials::graph::NodeKind;
    let mut to_visit = vec![node];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = to_visit.pop() {
        if !seen.insert(id) {
            continue;
        }
        match graph.node(id).map(|n| &n.kind) {
            Some(NodeKind::ImageTexture { texture, .. }) => return Some(*texture),
            Some(_) => to_visit.extend(graph.links().iter().filter(|l| l.to == id).map(|l| l.from)),
            None => {}
        }
    }
    None
}
