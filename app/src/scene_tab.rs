//! The Scene tab.
//!
//! `docs/04-UX-SPEC.md` §4.2: "expand/collapse, `Shift` range-select,
//! drag-to-reparent (with a clear insertion indicator), rename in place
//! (`F2`), and a search box that filters live and highlights matches while
//! keeping ancestors visible. Per-row: visibility eye, lock, material swatch."
//!
//! Every row's controls issue commands. None of them touch the document, so
//! every one of them is undoable and journalled without the panel knowing that
//! either of those things exist.

use crate::icons::{self, Icon};
use crate::project::Project;
use crate::theme;
use eframe::egui;
use tequila_scene::selection::{SelectionLevel, SelectionMode};
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::Command;

/// Panel state that is genuinely the panel's own — what is expanded, what is
/// being renamed — as distinct from the document, which the panel only reads.
#[derive(Debug, Default)]
pub struct SceneTab {
    pub search: String,
    collapsed: std::collections::BTreeSet<NodeId>,
    renaming: Option<(NodeId, String)>,
    /// The node being dragged, and the row it is currently over.
    dragging: Option<NodeId>,
    drop_target: Option<DropTarget>,
    /// A model to update from its file: the node it was asked on, and
    /// whether to ask for a different file. Drained by the app, which owns
    /// the background reads.
    pub update_request: Option<(NodeId, bool)>,
    /// A CAD model to divide again at another quality, drained likewise.
    pub retessellate_request: Option<(NodeId, tequila_import::step::Tessellation)>,
    /// A model whose units to ask about, drained likewise.
    pub units_request: Option<NodeId>,
    /// A model to improve, drained likewise.
    pub improve_request: Option<NodeId>,
    /// Where each row was drawn this frame, for the tests to aim at.
    #[cfg_attr(not(test), allow(dead_code))]
    rows: Vec<(NodeId, egui::Rect)>,
    /// Where the search's "Select all" was drawn, likewise.
    #[cfg_attr(not(test), allow(dead_code))]
    select_all_at: Option<egui::Rect>,
    /// The fields of what is selected, below the tree.
    pub metadata: crate::metadata_panel::MetadataPanel,
    /// The CAD layers, with the document and the history change they were
    /// listed at: listed every frame, they walked every node of an assembly
    /// to draw a list that had not changed.
    layers: Option<((u64, u64), Vec<tequila_scene::layers::Layer>)>,
    /// Where each layer's eye and name were drawn this frame, for the tests
    /// to aim at.
    #[cfg_attr(not(test), allow(dead_code))]
    layer_rows: Vec<(String, egui::Rect, egui::Rect)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DropTarget {
    node: NodeId,
    /// Onto the node itself (become its child), or between rows.
    inside: bool,
}

impl SceneTab {
    pub fn show(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        self.rows.clear();
        self.select_all_at = None;
        let response = theme::search_field(ui, &mut self.search, "Search parts, or Field = value");
        if response.changed() && !self.search.is_empty() {
            // A filtered tree with collapsed ancestors shows nothing, so
            // searching expands. §4.2: "keeping ancestors visible".
            self.collapsed.clear();
        }
        response.on_hover_text(
            "A name or a field's value; or by field (docs/03 §6.5):\n\
             Material = ABS   (wildcards: AB*)\n\
             Material != ABS\n\
             Finish: zinc   (contains)\n\
             PartNumber =   (has the field)",
        );
        if let Some(query) = tequila_scene::metadata::Query::parse(&self.search) {
            let matches = tequila_scene::metadata::find(project.document(), &query);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(match matches.len() {
                        0 => "No matches".to_string(),
                        1 => "1 match".to_string(),
                        n => format!("{n} matches"),
                    })
                    .small()
                    .color(theme::p().text_dim),
                );
                if !matches.is_empty() {
                    let button = ui.add(egui::Button::new("Select all").small()).on_hover_text("Select every match");
                    self.select_all_at = Some(button.rect);
                    if button.clicked() {
                        project.selection.select_many(matches, SelectionMode::Replace);
                    }
                }
            });
        }

        self.toolbar(ui, project);
        ui.add_space(2.0);

        // A search hides non-matching rows but keeps the ancestors of matches,
        // so a hit deep in an assembly is shown in its context rather than
        // ripped out of it.
        let visible = self.rows_to_show(project);

        // As tall as the tree, up to about half the window: a long
        // assembly scrolls in place, and the sections below - metadata,
        // transform, units - stay in sight rather than a scroll away.
        let most = (ui.ctx().screen_rect().height() * 0.45).max(220.0);
        // Only the rows scrolled into view are laid out: an assembly of ten
        // thousand parts laid out every row, every frame, three widgets
        // each, whether it was in view or not.
        egui::ScrollArea::vertical()
            .id_salt("scene-tree")
            .auto_shrink([false, true])
            .max_height(most)
            .show_rows(ui, ROW_HEIGHT, visible.len(), |ui, range| {
                let mut pending: Vec<Command> = Vec::new();
                let mut clicked: Option<(NodeId, SelectionMode)> = None;

                // Cleared each frame and re-established by whichever row is
                // hovered. Without this a target set on one frame outlives the
                // cursor leaving the tree, and releasing the button anywhere
                // reparents the node to wherever it last passed over.
                self.drop_target = None;

                for &id in &visible[range] {
                    self.row(ui, project, id, &mut pending, &mut clicked);
                }

                if let Some((id, mode)) = clicked {
                    // Taken and put back so the selection can be updated
                    // against the document it belongs to without borrowing
                    // the project twice.
                    let mut selection = std::mem::take(&mut project.selection);
                    selection.click(project.document(), id, mode);
                    project.selection = selection;
                }
                for command in pending {
                    if let Err(error) = project.apply(command) {
                        project.status = error.to_string();
                    }
                }
                self.finish_drag(ui, project);
            });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        ui.horizontal_wrapped(|ui| {
            let selected: Vec<NodeId> = project.selection.nodes().collect();
            let any = !selected.is_empty();

            // Adding a light is a scene-tree action, not a lighting-panel one,
            // because that is what a light *is*: a node, with a transform, a
            // place in the hierarchy, and a row here like everything else.
            let mut add_light = None;
            let mut add_cutaway = None;
            ui.menu_button("Add", |ui| {
            ui.menu_button("Light", |ui| {
                // Under the selected group when one is selected, so a light
                // added while a subassembly is open lands inside it rather
                // than at the root for the user to drag back.
                let parent = selected
                    .first()
                    .copied()
                    .filter(|id| {
                        project
                            .document()
                            .tree
                            .get(*id)
                            .is_some_and(|n| n.kind == NodeKind::Group)
                    });
                add_light = crate::lighting_tab::add_light_menu(ui, parent);
            });
            ui.menu_button("Cutaway", |ui| {
                let parent = selected.first().copied().filter(|id| {
                    project
                        .document()
                        .tree
                        .get(*id)
                        .is_some_and(|n| n.kind == NodeKind::Group)
                });
                add_cutaway = crate::cutaway_panel::add_cutaway_menu(ui, project, parent);
            });
            })
            .response
            .on_hover_text("Add a light or a cutaway to the scene");
            if let Some(command) = add_light {
                let _ = project.apply(command);
            }
            if let Some(command) = add_cutaway {
                let _ = project.apply(command);
            }

            if ui
                .add_enabled(selected.len() >= 2, egui::Button::new("Group"))
                .on_hover_text(crate::theme::keys("Ctrl+G"))
                .clicked()
            {
                let _ = project.apply(Command::Group {
                    nodes: selected.clone(),
                    name: "Group".into(),
                });
            }
            let groups: Vec<NodeId> = selected
                .iter()
                .copied()
                .filter(|id| {
                    project
                        .document()
                        .tree
                        .get(*id)
                        .is_some_and(|n| n.kind == NodeKind::Group)
                })
                .collect();
            if ui
                .add_enabled(!groups.is_empty(), egui::Button::new("Ungroup"))
                .clicked()
            {
                for group in groups {
                    let _ = project.apply(Command::Ungroup { group });
                }
            }
            if ui.add_enabled(any, egui::Button::new("Delete")).clicked() {
                let _ = project.apply(Command::DeleteNodes { nodes: selected });
            }
        });

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Select").color(theme::p().text_dim));
            let mut level = project.selection.level();
            let options: Vec<(SelectionLevel, &str)> = SelectionLevel::ALL.iter().map(|l| (*l, l.label())).collect();
            theme::segmented(ui, &mut level, &options);
            project.selection.set_level(level);
        });
    }

    /// Which rows the current search leaves visible.
    ///
    /// A match brings its ancestors with it: a hit six levels deep in an
    /// assembly is only useful if you can see where it lives.
    /// The rows to show, in the tree's order. Without a search, a walk
    /// that does not go into a collapsed group: asking every node for its
    /// ancestors instead was a list made per node per frame.
    fn rows_to_show(&self, project: &Project) -> Vec<NodeId> {
        let tree = &project.document().tree;
        if self.search.trim().is_empty() {
            let mut rows = Vec::new();
            let mut stack: Vec<NodeId> = tree.roots().iter().rev().copied().collect();
            while let Some(id) = stack.pop() {
                let Some(node) = tree.get(id) else { continue };
                rows.push(id);
                if !self.collapsed.contains(&id) {
                    stack.extend(node.children.iter().rev());
                }
            }
            return rows;
        }

        let Some(query) = tequila_scene::metadata::Query::parse(&self.search) else {
            return Vec::new();
        };
        let mut keep: std::collections::BTreeSet<NodeId> = Default::default();
        for id in tequila_scene::metadata::find(project.document(), &query) {
            keep.insert(id);
            keep.extend(tree.ancestors(id));
        }
        tree.depth_first().into_iter().filter(|id| keep.contains(id)).collect()
    }

    /// Which rays see the selected parts (`Rays`): what the camera,
    /// reflections, other surfaces and glass see of them, their shadows,
    /// and holdout. Folded away until wanted.
    pub fn rays_panel(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        let document = project.document();
        let selected: Vec<NodeId> = project
            .selection
            .nodes()
            .filter(|&id| document.tree.get(id).is_some_and(|n| !matches!(n.kind, tequila_scene::tree::NodeKind::Light { .. })))
            .collect();
        let Some(first) = selected.first().and_then(|&id| document.tree.get(id)) else { return };
        let before = first.rays;
        let mut rays = before;
        crate::field::Fold::new("Visible to")
            .id_salt("part-rays")
            .default_open(!before.is_default())
            .show(ui, |ui| {
                ui.add(crate::field::Checkbox::new(&mut rays.camera, "Camera"));
                ui.add(crate::field::Checkbox::new(&mut rays.glossy, "Reflections"))
                    .on_hover_text("Shows in other parts' reflections and the floor's. Off, and a prop stays in the picture without showing in the product's finish.");
                ui.add(crate::field::Checkbox::new(&mut rays.diffuse, "Light it bounces"))
                    .on_hover_text("Lights other surfaces by the light it bounces onto them.");
                ui.add(crate::field::Checkbox::new(&mut rays.transmission, "Through glass"));
                ui.add(crate::field::Checkbox::new(&mut rays.shadow, "Casts shadows"));
                ui.add(crate::field::Checkbox::new(&mut rays.holdout, "Holdout"))
                    .on_hover_text("Cut out of the picture where the camera sees it: nothing there, with no alpha, for compositing something in its place.");
                if selected.len() > 1 {
                    ui.label(egui::RichText::new(format!("For all {} selected", selected.len())).small().color(crate::theme::dim()));
                }
            });
        if rays != before {
            let _ = project.apply(tequila_scene::Command::SetRays { states: selected.iter().map(|&id| (id, rays)).collect() });
        }
    }

    /// The CAD layers the scene's parts are on (`tequila_scene::layers`): an
    /// eye each, which shows or hides every part on it as one undo step, and
    /// its name, which selects them. Nothing at all where no part is on a
    /// layer, which is every model but a CAD one.
    pub fn layers_panel(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        use tequila_scene::layers::Shown;
        self.layer_rows.clear();
        let counted_at = (project.document_epoch(), project.bus().changes());
        let layers = match &self.layers {
            Some((at, layers)) if *at == counted_at => layers.clone(),
            _ => {
                let layers = tequila_scene::layers::layers(project.document());
                self.layers = Some((counted_at, layers.clone()));
                layers
            }
        };
        if layers.is_empty() {
            return;
        }
        ui.add_space(4.0);
        ui.separator();
        theme::section(ui, "Layers");
        let mut command = None;
        let mut select = None;
        // An IGES file can have a hundred levels; they scroll rather than
        // push the sections below out of reach.
        egui::ScrollArea::vertical().id_salt("cad-layers").auto_shrink([false, true]).max_height(180.0).show(ui, |ui| {
            for layer in &layers {
                let shown = layer.shown();
                ui.horizontal(|ui| {
                    let eye = icons::button(ui, if shown == Shown::None { Icon::EyeClosed } else { Icon::Eye }, false, 20.0, egui::Sense::click())
                        .on_hover_text(match shown {
                            Shown::All => "Hide every part on this layer",
                            Shown::None => "Show every part on this layer",
                            Shown::Some => "Some of its parts are hidden: show them all",
                        });
                    if eye.clicked() {
                        // Partly shown reads as a layer to show: one click
                        // puts it back as the file had it.
                        command = Some(Command::SetLayerVisibility { layer: layer.name.clone(), visible: shown != Shown::All });
                    }
                    let mut text = egui::RichText::new(&layer.name);
                    text = match shown {
                        Shown::All => text.color(theme::p().text),
                        Shown::None => text.color(theme::p().text_faint),
                        Shown::Some => text.color(theme::p().text_dim).italics(),
                    };
                    let parts = layer.nodes.len();
                    let count = if parts == 1 { "1 part".to_string() } else { format!("{parts} parts") };
                    // The name gets what the count leaves, measured out by
                    // hand: a truncated label takes the whole row otherwise.
                    let width = (ui.available_width() - 64.0).max(40.0);
                    let name = ui
                        .allocate_ui_with_layout(egui::vec2(width, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.set_min_width(width);
                            ui.add(egui::Label::new(text).truncate().sense(egui::Sense::click()))
                        })
                        .inner
                        .on_hover_text(format!("{}: {count}. Click to select them.", layer.name));
                    if name.clicked() {
                        select = Some(layer.nodes.clone());
                    }
                    self.layer_rows.push((layer.name.clone(), eye.rect, name.rect));
                    ui.label(egui::RichText::new(count).small().color(theme::dim()));
                });
            }
        });
        if let Some(command) = command {
            if let Err(error) = project.apply(command) {
                project.status = error.to_string();
            }
        }
        if let Some(nodes) = select {
            project.selection.select_many(nodes, SelectionMode::Replace);
        }
    }

    /// The metadata panel, which shares the search box: a field clicked
    /// there becomes a search here.
    pub fn metadata_panel(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        let before = self.search.clone();
        self.metadata.show(ui, project, &mut self.search);
        if self.search != before && !self.search.is_empty() {
            self.collapsed.clear();
        }
    }

    fn row(
        &mut self,
        ui: &mut egui::Ui,
        project: &mut Project,
        id: NodeId,
        pending: &mut Vec<Command>,
        clicked: &mut Option<(NodeId, SelectionMode)>,
    ) {
        let Some(node) = project.document().tree.get(id) else {
            return;
        };
        let depth = project.document().tree.ancestors(id).len();
        let is_group = node.kind == NodeKind::Group;
        let has_children = !node.children.is_empty();
        let selected = project.selection.contains(id);
        let visible = node.visible;
        let locked = node.locked;
        let name = node.name.clone();
        let inherited_hidden = !project.document().tree.is_effectively_visible(id);

        let kind = match node.kind {
            NodeKind::Group => Icon::Group,
            NodeKind::Part { .. } => Icon::Part,
            NodeKind::Light { .. } => Icon::Light,
            NodeKind::Cutaway { .. } => Icon::Cutaway,
        };

        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click_and_drag(),
        );
        self.rows.push((id, rect));
        if !ui.is_rect_visible(rect) {
            // Off the scroll area's view: still a row to click and drop on
            // once it scrolls in, nothing to draw now.
        } else if selected {
            ui.painter().rect_filled(rect, 5.0, theme::accent_fill());
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 6.0, theme::p().hover.gamma_multiply(0.55));
        }
        let middle = rect.center().y;
        let mut x = rect.left() + 4.0 + depth as f32 * 14.0;

        // Expand / collapse.
        if has_children {
            let open = !self.collapsed.contains(&id);
            let at = egui::Rect::from_min_size(egui::pos2(x, middle - 9.0), egui::Vec2::splat(18.0));
            let toggle = ui.interact(at, ui.id().with(("tree-toggle", id)), egui::Sense::click());
            icons::paint(
                ui.painter(),
                at.shrink(4.0),
                if open { Icon::ChevronDown } else { Icon::ChevronRight },
                if toggle.hovered() { theme::p().text } else { theme::p().text_dim },
            );
            if toggle.on_hover_text(if open { "Collapse" } else { "Expand" }).clicked() {
                if open {
                    self.collapsed.insert(id);
                } else {
                    self.collapsed.remove(&id);
                }
            }
        }
        x += 18.0;
        let ink = if selected {
            theme::p().accent_text
        } else if inherited_hidden {
            theme::p().text_faint
        } else {
            theme::p().text_dim
        };
        icons::paint(ui.painter(), egui::Rect::from_min_size(egui::pos2(x + 1.0, middle - 7.0), egui::Vec2::splat(14.0)), kind, ink);
        x += 22.0;

        // The eye and the lock, at the row's end. Shown while the row is
        // under the pointer, and always when they are not at rest — a hidden
        // or locked part says so without being hovered, and a tree of a
        // thousand parts is not a column of a thousand eyes. A part hidden
        // by an ancestor shows its eye dimmed rather than closed, so "why is
        // this not rendering" has an answer on the row.
        let lock_at = egui::Rect::from_min_size(egui::pos2(rect.right() - 24.0, middle - 10.0), egui::Vec2::splat(20.0));
        let eye_at = lock_at.translate(egui::vec2(-22.0, 0.0));
        let row_hovered = response.hovered() || ui.rect_contains_pointer(rect);
        let eye = ui.interact(eye_at, ui.id().with(("tree-eye", id)), egui::Sense::click());
        if row_hovered || !visible || inherited_hidden {
            let colour = if eye.hovered() {
                theme::p().text
            } else if !visible || inherited_hidden {
                theme::p().text_faint
            } else {
                theme::p().text_dim
            };
            icons::paint(ui.painter(), eye_at.shrink(3.0), if visible { Icon::Eye } else { Icon::EyeClosed }, colour);
        }
        if eye.clicked() {
            pending.push(Command::set_visibility(id, !visible));
        }
        eye.on_hover_text(if inherited_hidden && visible {
            "Hidden by a parent"
        } else if visible {
            "Hide"
        } else {
            "Show"
        });
        let lock = ui.interact(lock_at, ui.id().with(("tree-lock", id)), egui::Sense::click());
        if row_hovered || locked {
            let colour = if lock.hovered() {
                theme::p().text
            } else if locked {
                theme::p().text_dim
            } else {
                theme::p().text_faint
            };
            icons::paint(ui.painter(), lock_at.shrink(3.0), if locked { Icon::Lock } else { Icon::Unlock }, colour);
        }
        if lock.clicked() {
            pending.push(Command::set_locked(id, !locked));
        }
        lock.on_hover_text(if locked { "Unlock" } else { "Lock" });

        let name_at = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(eye_at.left() - 4.0, rect.bottom()));

        // Rename in place (F2).
        if let Some((renaming, text)) = &mut self.renaming {
            if *renaming == id {
                let editor = ui.put(name_at.shrink2(egui::vec2(0.0, 2.0)), egui::TextEdit::singleline(text));
                editor.request_focus();
                let commit = editor.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if commit && !text.trim().is_empty() {
                    pending.push(Command::Rename {
                        node: id,
                        name: text.trim().to_string(),
                    });
                }
                if commit || cancel {
                    self.renaming = None;
                }
                return;
            }
        }

        if ui.is_rect_visible(rect) {
            let colour = if selected {
                theme::p().text
            } else if inherited_hidden {
                theme::p().text_faint
            } else {
                theme::p().text
            };
            // One line, ellipsised: a wrapped name makes rows of different
            // heights, and a part is found by the start of its name.
            let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), egui::FontId::proportional(13.0), colour);
            job.wrap = egui::text::TextWrapping {
                max_width: name_at.width().max(0.0),
                max_rows: 1,
                break_anywhere: true,
                overflow_character: Some('…'),
            };
            let galley = ui.fonts(|f| f.layout_job(job));
            ui.painter().galley(egui::pos2(name_at.left(), middle - galley.size().y * 0.5), galley, egui::Color32::PLACEHOLDER);
        }

        if response.clicked() {
            let modifiers = ui.input(|i| i.modifiers);
            *clicked = Some((
                id,
                if modifiers.shift && modifiers.command {
                    SelectionMode::Remove
                } else if modifiers.shift {
                    SelectionMode::Range
                } else if modifiers.command {
                    SelectionMode::Add
                } else {
                    SelectionMode::Replace
                },
            ));
        }
        if response.double_clicked() {
            self.renaming = Some((id, name.clone()));
        }
        if selected && ui.input(|i| i.key_pressed(egui::Key::F2)) && self.renaming.is_none() {
            self.renaming = Some((id, name.clone()));
        }

        // Drag to reparent. The insertion indicator §4.2 asks for is drawn as
        // a line across the target row.
        if response.drag_started() {
            self.dragging = Some(id);
        }
        // `contains_pointer`, not `hovered`: while a drag is under way egui
        // counts only the dragged row as hovered, so asking the row beneath
        // it whether it was hovered never found anywhere to drop.
        if self.dragging.is_some() && response.contains_pointer() {
            self.drop_target = Some(DropTarget {
                node: id,
                inside: is_group,
            });
        }
        if self.dragging.is_some_and(|d| d != id) {
            if let Some(target) = self.drop_target {
                if target.node == id {
                    let rect = response.rect;
                    let stroke = egui::Stroke::new(2.0_f32, theme::p().accent);
                    if target.inside {
                        ui.painter().rect_stroke(rect, 5.0, stroke);
                    } else {
                        ui.painter()
                            .line_segment([rect.left_bottom(), rect.right_bottom()], stroke);
                    }
                }
            }
        }

        response.context_menu(|ui| {
            // Split first, because it is the entry somebody is looking for
            // when they open this menu on a model that came in as one lump.
            // Counted before it is offered, so the label says how many parts
            // and the item greys out when there is only one — an action that
            // reports "already one part" after being clicked should have said
            // so before.
            let parts = Self::split_count(project, id);
            let can_split = parts > 1;
            let label = if can_split {
                format!("Split into {parts} parts")
            } else {
                "Split into parts".to_string()
            };
            if ui
                .add_enabled(can_split, egui::Button::new(label))
                .on_hover_text(
                    "Separate the mesh into its disconnected shells. For a model \
                     that arrived as one part because the file had no way to say \
                     otherwise.",
                )
                .on_disabled_hover_text(if is_group {
                    "Only a part can be split; this is a group."
                } else {
                    "This mesh is a single connected shell."
                })
                .clicked()
            {
                pending.push(Command::SplitIntoParts { node: id });
                ui.close_menu();
            }
            // The model this node came in with, updated from its file: the
            // same file, changed since, or a newer one saved elsewhere.
            let roots = tequila_scene::reimport::import_roots(project.document(), id);
            let file = tequila_scene::reimport::source_file(project.document(), &roots);
            if ui
                .add_enabled(file.is_some(), egui::Button::new("Reload from file"))
                .on_hover_text(format!(
                    "Read {} again and update the model from it. Parts keep their \
                     materials, labels, names and visibility; new parts are added and \
                     parts no longer in the file are hidden.",
                    file.as_deref().unwrap_or("the file")
                ))
                .on_disabled_hover_text("This model was imported before Tequila recorded where models come from. Use Update from another file.")
                .clicked()
            {
                self.update_request = Some((id, false));
                ui.close_menu();
            }
            if ui
                .button("Update from another file…")
                .on_hover_text("Update this model from a newer version saved under another name, keeping its materials and labels.")
                .clicked()
            {
                self.update_request = Some((id, true));
                ui.close_menu();
            }
            if ui
                .button("Improve model…")
                .on_hover_text(
                    "Smoother curves for a model that came in faceted: divide its CAD surfaces again, \
                     subdivide its mesh, or work out its shading again.",
                )
                .clicked()
            {
                self.improve_request = Some(id);
                ui.close_menu();
            }
            if ui
                .button("Units of this model…")
                .on_hover_text(
                    "Say what unit the model's numbers are in — millimetres, inches, metres — \
                     when it came in at the wrong size and its materials' patterns look far \
                     too fine or too coarse.",
                )
                .clicked()
            {
                self.units_request = Some(id);
                ui.close_menu();
            }
            // A quarter turn about its middle, staying on the floor: a CAD
            // model drawn Z up arrives lying on its back.
            ui.menu_button("Turn", |ui| {
                for turn in crate::orient::Turn::ALL {
                    if ui.button(turn.label()).on_hover_text(turn.hint()).clicked() {
                        if let Some(command) = crate::orient::turn(project.document(), &[id], turn) {
                            pending.push(command);
                        }
                        ui.close_menu();
                    }
                }
            });
            // A CAD model: its exact surfaces, divided again at another
            // quality, from the source kept with the project.
            let top = roots.first().and_then(|r| project.document().tree.get(*r));
            let current = top
                .and_then(|n| n.metadata.get(tequila_import::step::TESSELLATION))
                .and_then(|label| tequila_import::step::Tessellation::from_label(label));
            if let Some(current) = current {
                let kept = top
                    .and_then(|n| n.metadata.get(tequila_scene::document::BREP))
                    .is_some_and(|id| project.document().sources.get(id).is_some());
                ui.menu_button("Tessellation", |ui| {
                    ui.label(egui::RichText::new(if kept { "From the CAD data kept with the project" } else { "From the file it came from" }).small().color(theme::p().text_dim));
                    for quality in tequila_import::step::Tessellation::ALL {
                        // The one in use shows selected, and is not offered.
                        if ui
                            .add_enabled(quality != current, egui::SelectableLabel::new(quality == current, quality.label()))
                            .on_hover_text(match quality {
                                tequila_import::step::Tessellation::Draft => "Fewer triangles: a large assembly seen whole",
                                tequila_import::step::Tessellation::Medium => "The default: smooth at a normal distance",
                                tequila_import::step::Tessellation::High => "For close-ups of curved edges",
                                tequila_import::step::Tessellation::Ultra => "For extreme close-ups; many triangles",
                            })
                            .clicked()
                        {
                            self.retessellate_request = Some((id, quality));
                            ui.close_menu();
                        }
                    }
                });
            }
            ui.separator();

            // One command each, not one per node. On a 4,000-part assembly
            // the difference is a single undo step against four thousand —
            // and Isolate in particular has to be *one*, since it both shows
            // and hides, and two steps would make the first Ctrl+Z look like
            // it did nothing.
            if ui.button("Isolate").clicked() {
                let tree = &project.document().tree;
                let states = tree
                    .iter()
                    .map(|node| {
                        let keep = node.id == id
                            || tree.ancestors(id).contains(&node.id)
                            || tree.ancestors(node.id).contains(&id);
                        (node.id, keep)
                    })
                    .collect();
                pending.push(Command::SetVisibility { states });
                ui.close_menu();
            }
            if ui.button("Hide others").clicked() {
                let selected: Vec<NodeId> = project.selection.nodes().collect();
                let states = project
                    .document()
                    .tree
                    .iter()
                    .map(|node| (node.id, selected.contains(&node.id)))
                    .collect();
                pending.push(Command::SetVisibility { states });
                ui.close_menu();
            }
            if ui.button("Show all").clicked() {
                pending.push(Command::set_visibility_of(
                    project
                        .document()
                        .tree
                        .iter()
                        .map(|n| n.id)
                        .collect::<Vec<_>>(),
                    true,
                ));
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Rename").clicked() {
                self.renaming = Some((id, name.clone()));
                ui.close_menu();
            }
            if ui.button("Delete").clicked() {
                pending.push(Command::DeleteNodes { nodes: vec![id] });
                ui.close_menu();
            }
        });
    }

    /// How many separate shells a node's mesh is made of.
    ///
    /// Counted every frame the menu is open, which sounds careless and is not:
    /// the menu is open for one node at a time, and the alternative — caching
    /// it per node — is a cache that has to be invalidated on every geometry
    /// edit to avoid offering a split that no longer applies. A union-find over
    /// one mesh is cheap next to being wrong.
    fn split_count(project: &Project, id: NodeId) -> usize {
        let document = project.document();
        let Some(node) = document.tree.get(id) else {
            return 0;
        };
        let NodeKind::Part { mesh } = node.kind else {
            return 0;
        };
        document.geometry.get(mesh).map(pieces).unwrap_or(0)
    }

    /// Finish a drag-to-reparent when the button comes up.
    fn finish_drag(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        if !ui.input(|i| i.pointer.any_released()) {
            return;
        }
        let (Some(node), Some(target)) = (self.dragging.take(), self.drop_target.take()) else {
            self.dragging = None;
            self.drop_target = None;
            return;
        };
        if node == target.node {
            return;
        }

        let (parent, index) =
            match project
                .document()
                .tree
                .drop_position(node, target.node, target.inside)
            {
                Ok(position) => position,
                Err(error) => {
                    project.status = error.to_string();
                    return;
                }
            };
        if let Err(error) = project.apply(Command::Reparent {
            node,
            parent,
            index: Some(index),
        }) {
            project.status = error.to_string();
        }
    }
}

/// A row of the tree: every row the same height, so the scroll area can
/// lay out only the ones in view.
const ROW_HEIGHT: f32 = 26.0;

/// How many separate pieces `mesh` falls into, for the Split menu item.
///
/// Remembered for the mesh last asked about: a context menu asks every
/// frame it is open, and counting welds and walks every triangle, which on
/// a 700,000-triangle STL made the open menu lag. The mesh is known by its
/// arrays' addresses and lengths, which a rewritten mesh does not keep.
pub(crate) fn pieces(mesh: &tequila_scene::Mesh) -> usize {
    static LAST: std::sync::Mutex<Option<((usize, usize, usize), usize)>> = std::sync::Mutex::new(None);
    let identity = (mesh.positions.as_ptr() as usize, mesh.positions.len(), mesh.indices.len());
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match *last {
        Some((seen, count)) if seen == identity => count,
        _ => {
            let count = tequila_scene::split::part_count(mesh);
            *last = Some((identity, count));
            count
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Harness {
        ctx: egui::Context,
        tab: SceneTab,
        project: Project,
        time: f64,
    }

    impl Harness {
        fn new() -> Harness {
            let project = Project::with_document(
                tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2),
                std::sync::Arc::new(tequila_scene::demo::studio_environment()),
            );
            let mut h = Harness { ctx: egui::Context::default(), tab: SceneTab::default(), project, time: 0.0 };
            h.frame(Vec::new());
            h
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (tab, project) = (&mut self.tab, &mut self.project);
            let _ = self.ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| tab.show(ui, project));
            });
        }

        fn row(&self, id: NodeId) -> egui::Rect {
            self.tab.rows.iter().find(|(n, _)| *n == id).map(|(_, r)| *r).expect("the row is drawn")
        }

        fn press(&mut self, at: egui::Pos2, pressed: bool) {
            self.frame(vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
            ]);
        }

        fn parent(&self, id: NodeId) -> Option<NodeId> {
            self.project.document().tree.get(id).and_then(|n| n.parent)
        }
    }

    /// While a part is dragged over a group's row, that row is the target —
    /// the insertion indicator §4.2 asks for is drawn from it — and the drop
    /// lands the part inside the group.
    ///
    /// The target was taken from whichever row was hovered, and while a drag
    /// is under way egui hovers nothing but the row being dragged: the target
    /// was the dragged row itself, so no indicator ever showed. The drop only
    /// worked because on the release frame the drag is over before the rows
    /// are asked.
    #[test]
    fn a_part_dragged_onto_a_group_moves_into_it() {
        let mut h = Harness::new();
        let tree = &h.project.document().tree;
        let demo = tree.iter().find(|n| n.kind == NodeKind::Group).map(|n| n.id).expect("the demo group");
        let part = tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| n.id).expect("a part");
        // Put the part one level further down, so there is somewhere to
        // drag it back to.
        h.project
            .apply(Command::Group { nodes: vec![part], name: "Inner".into() })
            .expect("grouped");
        let inner = h.parent(part).expect("the new group");
        assert_ne!(inner, demo);
        h.frame(Vec::new());
        h.frame(Vec::new());

        let from = h.row(part).center();
        let to = h.row(demo).center();
        h.press(from, true);
        for step in 1..=8 {
            let at = from + (to - from) * (step as f32 / 8.0);
            h.frame(vec![egui::Event::PointerMoved(at)]);
        }
        assert_eq!(h.tab.drop_target, Some(DropTarget { node: demo, inside: true }), "the group under the pointer is the target mid-drag");
        h.press(to, false);
        h.frame(Vec::new());
        assert_eq!(h.parent(part), Some(demo), "the part moved into the group it was dropped on");
    }

    fn first_part(h: &Harness) -> NodeId {
        h.project
            .document()
            .tree
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Part { .. }))
            .map(|n| n.id)
            .expect("a part")
    }

    /// Typing `Field = value` in the search shows the parts with it, and
    /// "Select all" selects them.
    #[test]
    fn a_field_search_filters_and_selects() {
        let mut h = Harness::new();
        let part = first_part(&h);
        h.project
            .apply(Command::SetMetadata { node: part, key: "Material".into(), value: Some("ABS".into()) })
            .unwrap();
        h.tab.search = "material = abs".into();
        h.frame(Vec::new());
        let drawn = |h: &Harness| h.tab.rows.iter().map(|(n, _)| *n).collect::<Vec<NodeId>>();
        assert!(drawn(&h).contains(&part), "the matching part is shown");
        h.tab.search = "Material = PC".into();
        h.frame(Vec::new());
        assert!(!drawn(&h).contains(&part), "a part whose field differs is not");
        assert!(h.tab.select_all_at.is_none(), "nothing to select");
        h.tab.search = "Material = A*".into();
        h.frame(Vec::new());
        let at = h.tab.select_all_at.expect("the button is drawn").center();
        h.frame(vec![egui::Event::PointerMoved(at)]);
        h.press(at, true);
        h.press(at, false);
        h.frame(Vec::new());
        assert_eq!(h.project.selection.nodes().collect::<Vec<_>>(), vec![part], "Select all selected the match");
    }

    /// A model of two parts on CAD layers, one of them switched off, and a
    /// part on none.
    fn layered() -> Project {
        use tequila_scene::command::{reserve_keys, ImportPayload};
        let mut document = tequila_scene::SceneDocument::new();
        let mesh = tequila_scene::Mesh::new(
            vec![tequila_core::math::Vec3::ZERO, tequila_core::math::vec3(1.0, 0.0, 0.0), tequila_core::math::vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        );
        let (meshes, _, ids) = reserve_keys(&document, 1, 0, 4);
        let mut root = tequila_scene::SceneNode::new(ids[0], "Model", NodeKind::Group);
        root.children = ids[1..].to_vec();
        let mut nodes = vec![root];
        for (index, name, layer) in [(1, "Box", Some("Outer")), (2, "Pin", Some("Construction")), (3, "Screw", None)] {
            let mut node = tequila_scene::SceneNode::new(ids[index], name, NodeKind::Part { mesh: meshes[0] });
            node.parent = Some(ids[0]);
            node.layers = layer.into_iter().map(String::from).collect();
            node.visible = name != "Pin";
            nodes.push(node);
        }
        let payload = ImportPayload { meshes: vec![(meshes[0], mesh)], textures: Vec::new(), materials: Vec::new(), nodes, roots: vec![ids[0]] };
        tequila_scene::CommandBus::new().apply(&mut document, Command::Import { payload: Box::new(payload) }).unwrap();
        Project::with_document(document, std::sync::Arc::new(tequila_scene::demo::studio_environment()))
    }

    /// The Layers list has a row a layer; its eye shows a switched-off one
    /// (one undo step) and hides it again, and its name selects its parts.
    /// A scene with no layers draws no list.
    #[test]
    fn the_layers_list_shows_hides_and_selects_by_layer() {
        let ctx = egui::Context::default();
        let mut tab = SceneTab::default();
        let mut project = layered();
        let mut time = 0.0;
        let mut frame = |tab: &mut SceneTab, project: &mut Project, events: Vec<egui::Event>| {
            time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 700.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| tab.layers_panel(ui, project));
            });
        };
        let button = |at: egui::Pos2, pressed: bool| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
            ]
        };
        let mut click = |tab: &mut SceneTab, project: &mut Project, at: egui::Pos2| {
            frame(tab, project, vec![egui::Event::PointerMoved(at)]);
            frame(tab, project, button(at, true));
            frame(tab, project, button(at, false));
            frame(tab, project, Vec::new());
        };
        let pin = |project: &Project| project.document().tree.iter().find(|n| n.name == "Pin").unwrap().clone();
        click(&mut tab, &mut project, egui::pos2(-10.0, -10.0));
        let names: Vec<&str> = tab.layer_rows.iter().map(|(name, ..)| name.as_str()).collect();
        assert_eq!(names, ["Outer", "Construction"]);

        let eye = tab.layer_rows[1].1.center();
        click(&mut tab, &mut project, eye);
        assert!(pin(&project).visible, "the eye showed the switched-off layer");
        assert_eq!(project.bus().undo_name(), Some("scene.set_layer_visibility"));
        let eye = tab.layer_rows[1].1.center();
        click(&mut tab, &mut project, eye);
        assert!(!pin(&project).visible, "and hid it again");

        let name = tab.layer_rows[0].2.center();
        click(&mut tab, &mut project, name);
        let boxed = project.document().tree.iter().find(|n| n.name == "Box").unwrap().id;
        assert_eq!(project.selection.nodes().collect::<Vec<_>>(), vec![boxed], "the name selected the layer's part");

        let mut plain = Project::with_document(
            tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2),
            std::sync::Arc::new(tequila_scene::demo::studio_environment()),
        );
        let mut fresh = SceneTab::default();
        click(&mut fresh, &mut plain, egui::pos2(-10.0, -10.0));
        assert!(fresh.layer_rows.is_empty(), "a scene without layers has no list");
    }

    /// The eye on a row hides the part, without selecting it.
    #[test]
    fn the_eye_hides_without_selecting() {
        let mut h = Harness::new();
        let part = h
            .project
            .document()
            .tree
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Part { .. }))
            .map(|n| n.id)
            .expect("a part");
        let row = h.row(part);
        let eye = egui::pos2(row.right() - 24.0 - 22.0 + 10.0, row.center().y);
        h.frame(vec![egui::Event::PointerMoved(eye)]);
        h.press(eye, true);
        h.press(eye, false);
        h.frame(Vec::new());
        assert!(!h.project.document().tree.get(part).unwrap().visible, "hidden");
        assert!(!h.project.selection.contains(part), "and not selected by the click");
    }
}
