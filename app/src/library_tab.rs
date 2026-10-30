//! The material library browser (`docs/04-UX-SPEC.md` §9).
//!
//! ## Thumbnails are rendered, and rendered lazily
//!
//! §9 asks for previews "rendered by the actual renderer ... generated lazily on
//! first display, cached ... keyed by content hash". Both halves matter and they
//! pull against each other.
//!
//! Rendering them for real is what makes a thumbnail a promise rather than an
//! illustration — a library of approximations teaches people to ignore it and
//! apply materials one at a time to find out what they look like. But path
//! tracing sixty spheres the moment a window opens is a visible stall, and a
//! browser that hitches when you scroll is worse than one with plain swatches.
//!
//! A single background job renders previews; the UI only uploads completed
//! images. Only tiles on screen, and the next screenful, are rendered, top
//! first, and they are cached by content hash — in memory, and on disk for
//! every later launch, so the library is rendered once rather than on every
//! start or every update. Pending tiles show a neutral placeholder.

use eframe::egui;
use tequila_format::library::{self, LibraryEntry};
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_cpu::{render_thumbnail_textured, Device, PreviewSettings};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

type PreviewJob = std::thread::JoinHandle<Option<(u64, egui::ColorImage)>>;

/// How long a frame may spend reading kept thumbnails from disk before it
/// leaves the rest for the next, so opening the library never hitches.
const DISK_BUDGET: std::time::Duration = std::time::Duration::from_millis(8);

/// The browser's width. Fixed, not a maximum — see the note where the panel is
/// created.
///
/// **Not covered by a test, and it is worth saying so.** The failure this
/// guards against is a feedback loop between egui's content-driven panel sizing
/// and a grid that measures itself against the space it is offered, and it
/// cannot be reproduced headlessly: pinning the width in a test harness is
/// exactly what stops the loop from happening. The guarantee here is structural
/// — a panel created with `exact_width` has no mechanism by which to grow — not
/// a test. [`columns_for`] is the half that *is* testable, and is tested.
pub const PANEL_WIDTH: f32 = 260.0;

/// The widest the browser's contents may lay themselves out.
///
/// Kept slightly under [`PANEL_WIDTH`] so the grid never asks for the last few
/// pixels and starts a fight with the scroll bar.
pub const MAX_WIDTH: f32 = PANEL_WIDTH - 16.0;

/// A virtual folder, in the order §9 lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Shelf {
    #[default]
    All,
    Favorites,
    Recent,
    /// Materials already in the open document.
    InProject,
    /// The user's own: the ones they made, imported or copied.
    Mine,
}

impl Shelf {
    pub const ALL: [Shelf; 5] = [
        Shelf::All,
        Shelf::Mine,
        Shelf::Favorites,
        Shelf::Recent,
        Shelf::InProject,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Shelf::All => "All",
            Shelf::Favorites => "Favorites",
            Shelf::Recent => "Recent",
            Shelf::InProject => "In project",
            Shelf::Mine => "Mine",
        }
    }
}

/// What the user picked, for the caller to act on. Entries are named by
/// their file: an index would point at another entry the moment the
/// library is read again.
#[derive(Clone, Debug)]
pub enum LibraryAction {
    /// Remove a user entry, asked and confirmed. The browser has already
    /// checked it is the user's own; the guard in `tequila-format` checks
    /// again.
    Delete(String),
    /// Make a material, as the New material dialog describes it.
    Create(Box<crate::library_dialogs::NewMaterial>),
    /// A copy in the user's own library, to change.
    Duplicate(String),
    /// New name, category and tags for one of the user's own.
    EditDetails(crate::library_dialogs::Details),
    /// One of the user's own, replaced by the selected part's material.
    UpdateFromSelection(String),
    /// A material package of an entry, to save somewhere.
    Export(String),
    /// A material file, into the user's library.
    ImportFile,
    /// The user's library folder, opened.
    OpenFolder,
    /// Apply to the current selection (§3.3: double-click with a selection).
    ApplyToSelection(Box<LibraryEntry>),
    /// A drag began; the viewport handles the drop.
    BeganDrag(Box<LibraryEntry>),
    /// Make materials from a downloaded texture pack: `true` to pick a
    /// folder, `false` a ZIP or one of its maps.
    ImportTexturePack(bool),
    /// Something asked of the Studios section.
    Studio(crate::studio_shelf::StudioAction),
}

/// What the browser shows: materials, or whole studios.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Materials,
    Studios,
}

pub struct LibraryTab {
    entries: Vec<LibraryEntry>,
    /// Each entry's [`entry_key`], worked out when the entries are read.
    ///
    /// Asked for every tile on every frame, and not cheap: it formats every
    /// parameter of the material and hashes its whole graph. Done per frame
    /// it was most of the application's frame time — 16 of 17 ms with the
    /// procedural library open.
    keys: Vec<u64>,
    problems: Vec<String>,
    query: String,
    shelf: Shelf,
    category: Option<String>,
    favorites: BTreeSet<String>,
    recent: Vec<String>,
    /// Where the two shelves are kept between launches; none in a test.
    /// Kept in memory only, they were empty after every restart and every
    /// update.
    shelf_files: Option<ShelfFiles>,
    /// Rendered previews, keyed by the material's content.
    thumbnails: HashMap<u64, egui::TextureHandle>,
    /// How many pixels across a tile is drawn, so how many its preview is
    /// rendered at ([`tile_pixels`]).
    tile_pixels: usize,
    /// Names still waiting for a preview, in the order they were first seen.
    queued: Vec<usize>,
    preview_job: Option<PreviewJob>,
    device: Option<Arc<Device>>,
    environment: Arc<Environment>,
    /// Thumbnails kept from earlier launches.
    disk: Arc<crate::thumbnail::DiskCache>,
    pub thumbnail_size: f32,
    new_material: crate::library_dialogs::NewMaterialDialog,
    details: crate::library_dialogs::DetailsDialog,
    confirm_delete: crate::library_dialogs::ConfirmDelete,
    /// When the user's folder was last looked at, and what was in it: a
    /// preset dropped into the folder by hand, or by another copy of the
    /// application, shows up without a restart.
    watched: (std::time::Instant, u64),
    /// The enabled plugins' material folders, and the plugins' names.
    plugin_folders: Vec<(std::path::PathBuf, String)>,
    pub section: Section,
    /// Studio presets, the other section.
    pub studios: crate::studio_shelf::StudioShelf,
}

impl LibraryTab {
    pub fn new(environment: Arc<Environment>) -> LibraryTab {
        let mut tab = LibraryTab::with_cache(environment, Arc::new(crate::thumbnail::DiskCache::persistent()));
        tab.keep_shelves(ShelfFiles::beside_library());
        tab
    }

    /// Read the Favorites and Recent shelves from `files`, and write them
    /// back there whenever they change.
    pub fn keep_shelves(&mut self, files: ShelfFiles) {
        self.favorites = read_names(&files.favorites).into_iter().collect();
        self.recent = read_names(&files.recent);
        self.recent.truncate(RECENT);
        self.shelf_files = Some(files);
    }

    fn toggle_favorite(&mut self, name: &str) {
        if !self.favorites.remove(name) {
            self.favorites.insert(name.to_string());
        }
        if let Some(files) = &self.shelf_files {
            write_names(&files.favorites, self.favorites.iter());
        }
    }

    /// The same, keeping thumbnails in `disk`.
    pub fn with_cache(environment: Arc<Environment>, disk: Arc<crate::thumbnail::DiskCache>) -> LibraryTab {
        let (mut entries, mut problems) =
            library::read_directory_from(&standard_library_path(), library::Source::Standard);
        // The user's own, appended and then sorted with the rest — one shelf,
        // not two. A material somebody saved is a material, and putting it in
        // a separate list would mean remembering which list a thing is in
        // before being able to look for it.
        let (mine, mut mine_problems) =
            library::read_directory_from(&user_library_path(), library::Source::User);
        entries.extend(mine);
        problems.append(&mut mine_problems);
        entries.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
        LibraryTab {
            keys: entries.iter().map(entry_key).collect(),
            entries,
            problems,
            query: String::new(),
            shelf: Shelf::All,
            category: None,
            favorites: BTreeSet::new(),
            recent: Vec::new(),
            shelf_files: None,
            thumbnails: HashMap::new(),
            tile_pixels: 96,
            queued: Vec::new(),
            preview_job: None,
            device: Device::shared().ok(),
            environment,
            disk,
            thumbnail_size: 96.0,
            new_material: Default::default(),
            details: Default::default(),
            confirm_delete: Default::default(),
            watched: (std::time::Instant::now(), folder_signature(&user_library_path())),
            plugin_folders: Vec::new(),
            section: Section::Materials,
            studios: Default::default(),
        }
    }

    /// Read the studio presets again, the next time they are shown.
    pub fn reload_studios(&mut self) {
        self.studios.reload();
    }

    /// The enabled plugins' materials: read again when the set changes.
    pub fn set_plugin_folders(&mut self, folders: Vec<(std::path::PathBuf, String)>) {
        if folders != self.plugin_folders {
            self.plugin_folders = folders;
            self.reload();
        }
    }

    /// Open the New material dialog.
    pub fn new_material(&mut self, selected: Option<&crate::library_dialogs::Selected>) {
        self.new_material.open(selected);
    }

    /// Open the details dialog on the entry saved at `path`.
    pub fn edit(&mut self, path: &str) {
        if let Some(entry) = self.entries.iter().find(|e| e.path == path) {
            self.details.open(entry);
        }
    }

    /// Show the user's own materials.
    pub fn show_mine(&mut self) {
        self.shelf = Shelf::Mine;
        self.category = None;
        self.query.clear();
    }

    /// Re-read the user's folder if what is in it changed, at most every
    /// couple of seconds - a directory listing, not a re-read, when nothing
    /// did.
    pub fn watch(&mut self) {
        if self.watched.0.elapsed() < std::time::Duration::from_secs(2) {
            return;
        }
        let signature = folder_signature(&user_library_path());
        if signature != self.watched.1 {
            self.reload();
        }
        self.watched = (std::time::Instant::now(), signature);
    }

    pub fn entries(&self) -> &[LibraryEntry] {
        &self.entries
    }

    /// Re-read both directories, keeping the thumbnails that are still valid.
    ///
    /// Called after the user's library changes on disk. The thumbnail cache is
    /// keyed by the material's *content*, so a re-read costs nothing for
    /// entries that did not change — which is all of them but one.
    pub fn reload(&mut self) {
        let (mut entries, mut problems) =
            library::read_directory_from(&standard_library_path(), library::Source::Standard);
        let (mine, mut mine_problems) =
            library::read_directory_from(&user_library_path(), library::Source::User);
        entries.extend(mine);
        problems.append(&mut mine_problems);
        // A plugin's materials, found by its name as well as their own.
        for (folder, plugin) in &self.plugin_folders {
            let (theirs, mut their_problems) = library::read_directory_from(folder, library::Source::Plugin);
            entries.extend(theirs.into_iter().map(|mut entry| {
                entry.tags.push(plugin.clone());
                entry
            }));
            problems.append(&mut their_problems);
        }
        entries.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
        self.keys = entries.iter().map(entry_key).collect();
        self.entries = entries;
        self.problems = problems;
        self.queued.clear();
        self.watched = (std::time::Instant::now(), folder_signature(&user_library_path()));
    }

    /// Record that a material was applied, for the Recent shelf.
    pub fn note_used(&mut self, name: &str) {
        self.recent.retain(|n| n != name);
        self.recent.insert(0, name.to_string());
        self.recent.truncate(RECENT);
        if let Some(files) = &self.shelf_files {
            write_names(&files.recent, self.recent.iter());
        }
    }

    fn visible(&self, in_project: &BTreeSet<String>) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.matches(&self.query))
            .filter(|(_, e)| self.category.as_ref().is_none_or(|c| &e.category == c))
            .filter(|(_, e)| match self.shelf {
                Shelf::All => true,
                Shelf::Favorites => self.favorites.contains(&e.name),
                Shelf::Recent => self.recent.contains(&e.name),
                Shelf::InProject => in_project.contains(&e.name),
                Shelf::Mine => e.source == library::Source::User,
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        in_project: &BTreeSet<String>,
        selected: Option<&crate::library_dialogs::Selected>,
    ) -> Option<LibraryAction> {
        crate::theme::segmented(ui, &mut self.section, &[(Section::Materials, "Materials"), (Section::Studios, "Studios")]);
        ui.add_space(2.0);
        if self.section == Section::Studios {
            return self.studios.show(ui).map(LibraryAction::Studio);
        }
        let mut action = self.dialogs(ui.ctx(), selected);

        if !self.problems.is_empty() {
            // One bad preset must not leave the browser empty, but the person
            // who can fix it needs to know which one it was.
            ui.colored_label(
                crate::theme::p().busy,
                format!("{} preset(s) could not be loaded", self.problems.len()),
            )
            .on_hover_text(self.problems.join("\n"));
        }

        crate::theme::search_field(ui, &mut self.query, "Search materials");
        let shelves: Vec<(Shelf, &str)> = Shelf::ALL.iter().map(|s| (*s, s.label())).collect();
        crate::theme::segmented(ui, &mut self.shelf, &shelves);

        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("material-category")
                .width(ui.available_width() - 76.0)
                .selected_text(self.category.as_deref().unwrap_or("All categories"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.category, None, "All categories");
                    for category in library::categories(&self.entries) {
                        ui.selectable_value(&mut self.category, Some(category.clone()), category);
                    }
                });
            // The rest of the browser's settings, and the way in for
            // downloaded materials: the place people look for materials is
            // also the place they bring their own in.
            let new = crate::icons::tip(
                crate::icons::button(ui, crate::icons::Icon::Plus, false, 30.0, egui::Sense::click()),
                "New material",
                "Start from a type, the selected part, a texture pack or a file",
                "",
            );
            if new.clicked() {
                self.new_material.open(selected);
            }
            let more = crate::icons::button(ui, crate::icons::Icon::More, false, 30.0, egui::Sense::click())
                .on_hover_text("Preview size, your library's folder, and importing");
            let popup = ui.id().with("library-more");
            if more.clicked() {
                ui.memory_mut(|m| m.toggle_popup(popup));
            }
            egui::popup::popup_below_widget(ui, popup, &more, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                ui.set_min_width(200.0);
                ui.set_max_width(240.0);
                crate::theme::section(ui, "Previews");
                ui.add(
                    crate::field::Slider::new(&mut self.thumbnail_size, 40.0..=128.0)
                        .text("Size")
                        .show_value(false),
                );
                crate::theme::section(ui, "Your library");
                if crate::icons::labelled(ui, crate::icons::Icon::Upload, "Import a material file…", false).clicked() {
                    action = Some(LibraryAction::ImportFile);
                    ui.memory_mut(|m| m.close_popup());
                }
                if crate::icons::labelled(ui, crate::icons::Icon::FolderOpen, "Open the folder", false)
                    .on_hover_text("Presets dropped in here show up by themselves")
                    .clicked()
                {
                    action = Some(LibraryAction::OpenFolder);
                    ui.memory_mut(|m| m.close_popup());
                }
                if crate::icons::labelled(ui, crate::icons::Icon::Refresh, "Read it again", false).clicked() {
                    self.reload();
                    ui.memory_mut(|m| m.close_popup());
                }
                crate::theme::section(ui, "Import texture pack");
                if let Some(folder) = crate::texture_pack::menu(ui) {
                    action = Some(LibraryAction::ImportTexturePack(folder));
                    ui.memory_mut(|m| m.close_popup());
                }
            });
        });

        let visible = self.visible(in_project);
        ui.label(
            egui::RichText::new(format!("{} materials", visible.len()))
                .size(11.0)
                .color(crate::theme::p().text_faint),
        );
        if visible.is_empty() {
            use crate::icons::Icon;
            use crate::theme::{hue, Hue};
            let (icon, colour, title, hint) = match self.shelf {
                Shelf::Favorites => (Icon::Star, hue(Hue::Amber), "No favorites yet", "Click the star on a material to keep it here."),
                Shelf::Recent => (Icon::Clock, hue(Hue::Sky), "Nothing applied yet", "Materials you drag onto a part show up here."),
                Shelf::InProject => (Icon::Package, hue(Hue::Blue), "None in this project", "Library materials on the model's parts show up here."),
                Shelf::Mine => (Icon::Plus, hue(Hue::Green), "None of your own yet", "Click + to make one from a type, a part, a texture pack or a file, or drop a .fsmat here."),
                Shelf::All => (Icon::SearchX, hue(Hue::Slate), "Nothing matches", "Try another word, or every category."),
            };
            crate::theme::empty_state(ui, icon, colour, title, hint);
            return action;
        }

        // The grid's width is pinned to the panel's, not derived from
        // `available_width`.
        //
        // A resizable side panel grows to fit whatever its contents ask for,
        // and a grid that sizes itself from the space available asks for more
        // every time it is given more. The two together are a feedback loop:
        // the panel widens, the grid adds a column, the grid asks for more
        // width, the panel widens again — until the library has eaten the
        // window and the viewport has none left. Clamping here breaks it.
        let width = ui.available_width().min(MAX_WIDTH);
        ui.set_max_width(width);
        let columns = columns_for(width, self.thumbnail_size);
        // The preview size picks how many fit across; the tiles then share
        // the width out between them, so the grid runs edge to edge rather
        // than leaving a ragged strip down the right.
        let gap = 8.0;
        let size = ((width - gap * (columns as f32 - 1.0)) / columns as f32).floor().max(24.0);
        self.tile_pixels = tile_pixels(size, ui.ctx().pixels_per_point());

        let mut favourite_toggle: Option<String> = None;
        // Only the rows in view are laid out: every tile is a handful of
        // widgets, a tooltip and a context menu, and a library of a hundred
        // and fifty laid out in full every frame is time spent on tiles
        // nobody can see.
        let label_height = ui.fonts(|f| f.row_height(&egui::FontId::proportional(12.0)));
        let row_height = size + 4.0 + label_height;
        let rows = visible.len().div_ceil(columns);
        ui.spacing_mut().item_spacing.y = 10.0;
        // Which of `visible` are on screen, for the previews below.
        let mut in_view = 0..0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_height, rows, |ui, range| {
            ui.set_max_width(width);
            in_view = (range.start * columns).min(visible.len())..(range.end * columns).min(visible.len());
            for row in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for &index in visible.iter().skip(row * columns).take(columns) {
                        let entry = &self.entries[index];
                        let key = self.keys[index];
                        let starred = self.favorites.contains(&entry.name);
                        let used = in_project.contains(&entry.name);
                        let response = ui
                            .vertical(|ui| {
                                // `set_width` is a request, not a limit: a child
                                // wider than it — a long material name — pushes
                                // straight through, the grid's column grows to
                                // hold it, the panel grows to hold the grid, and
                                // the viewport is squeezed out of the window.
                                // `set_max_width` is the one that actually binds.
                                ui.set_width(size);
                                ui.set_max_width(size);
                                ui.spacing_mut().item_spacing.y = 4.0;

                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(size, size),
                                    egui::Sense::hover(),
                                );
                                let hovered = ui.rect_contains_pointer(rect);
                                match self.thumbnails.get(&key) {
                                    Some(texture) => {
                                        egui::Image::new(texture)
                                            .rounding(egui::Rounding::same(7.0))
                                            .paint_at(ui, rect);
                                    }
                                    None => {
                                        // A placeholder that reads as "not ready
                                        // yet" rather than as a black material:
                                        // the outline of the sphere to come.
                                        ui.painter().rect_filled(rect, 7.0, crate::theme::p().control);
                                        ui.painter().circle_stroke(
                                            rect.center(),
                                            size * 0.3,
                                            egui::Stroke::new(1.0_f32, crate::theme::p().line_strong),
                                        );
                                    }
                                }
                                if hovered {
                                    ui.painter().rect_stroke(
                                        rect,
                                        7.0,
                                        egui::Stroke::new(2.0_f32, crate::theme::p().accent),
                                    );
                                }
                                thumbnail_badges(ui, rect, starred, used);

                                // The name gets the whole tile width. It used to
                                // share the row with the favourite button, which
                                // on a 64 px tile left about forty pixels and
                                // turned every name into "Aluminiu…". The star is
                                // an overlay on the thumbnail now, where it costs
                                // no width at all.
                                //
                                // Truncated rather than wrapped: a wrapped name
                                // makes rows of different heights, and a name is
                                // identifiable from its first words. The full one
                                // is in the tooltip.
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&entry.name)
                                            .size(12.0)
                                            .color(if hovered { crate::theme::p().text } else { crate::theme::p().text_dim }),
                                    )
                                    .truncate(),
                                );
                            })
                            .response
                            .interact(egui::Sense::click_and_drag());

                        // The star is claimed from the tile's own response rather
                        // than being a nested button, so it cannot steal the drag
                        // that starts a material assignment — which is the tile's
                        // primary gesture and must never be blocked by a target
                        // sitting on top of it.
                        let star = star_rect(response.rect, size);
                        let on_star = response.hover_pos().is_some_and(|p| star.contains(p));
                        if on_star && response.clicked() {
                            favourite_toggle = Some(entry.name.clone());
                            action = None;
                            continue;
                        }

                        let editable = entry.is_editable();
                        let path = entry.path.clone();
                        let entry_name = entry.name.clone();
                        let has_selection = selected.is_some();
                        let mut ask_details = false;
                        let mut ask_delete = false;
                        response.context_menu(|ui| {
                            if ui.button("Apply to selection").clicked() {
                                action = Some(LibraryAction::ApplyToSelection(Box::new(
                                    self.entries[index].clone(),
                                )));
                                ui.close_menu();
                            }
                            if ui
                                .button(if starred {
                                    "Remove from favourites"
                                } else {
                                    "Add to favourites"
                                })
                                .clicked()
                            {
                                favourite_toggle = Some(entry_name.clone());
                                ui.close_menu();
                            }
                            ui.separator();
                            if crate::icons::labelled(ui, crate::icons::Icon::Copy, "Duplicate to my library", false).clicked() {
                                action = Some(LibraryAction::Duplicate(path.clone()));
                                ui.close_menu();
                            }
                            if editable {
                                if crate::icons::labelled(ui, crate::icons::Icon::Pencil, "Edit details…", false).clicked() {
                                    ask_details = true;
                                    ui.close_menu();
                                }
                                let update = ui
                                    .add_enabled(has_selection, egui::Button::new("Update from the selected part"))
                                    .on_hover_text("Replace this preset's material with the one on the selected part, keeping its name")
                                    .on_disabled_hover_text("Select a part whose material you tuned");
                                if update.clicked() {
                                    action = Some(LibraryAction::UpdateFromSelection(path.clone()));
                                    ui.close_menu();
                                }
                            }
                            if crate::icons::labelled(ui, crate::icons::Icon::Download, "Export…", false)
                                .on_hover_text("Save it as a material package (.fsmat) to share")
                                .clicked()
                            {
                                action = Some(LibraryAction::Export(path.clone()));
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Show file").on_hover_text(path.clone()).clicked() {
                                reveal(&path);
                                ui.close_menu();
                            }
                            // Delete is offered only for the user's own. The
                            // shipped library is part of the installation: it is
                            // the same for everybody and it is what the
                            // documentation describes, so an application that let
                            // somebody delete from it would be an application that
                            // could not be described.
                            if ui
                                .add_enabled(editable, egui::Button::new("Delete…"))
                                .on_disabled_hover_text(
                                    "Shipped with Tequila or a plugin, and read-only: duplicate it to change it.",
                                )
                                .clicked()
                            {
                                ask_delete = true;
                                ui.close_menu();
                            }
                        });
                        if ask_details {
                            self.details.open(&self.entries[index]);
                        }
                        if ask_delete {
                            self.confirm_delete.ask(&self.entries[index]);
                        }

                        let hover = format!(
                            "{}\n{} · {}\nDouble-click to apply to the selection, or drag \
                         onto a part.",
                            entry.name,
                            entry.material_type.label(),
                            entry.category
                        );
                        let response = response.on_hover_text(hover);

                        if response.double_clicked() {
                            action = Some(LibraryAction::ApplyToSelection(Box::new(entry.clone())));
                        } else if response.drag_started() {
                            action = Some(LibraryAction::BeganDrag(Box::new(entry.clone())));
                        }
                    }
                });
            }
        });

        // Applied after the grid, not inside it: the closure holds `&self.entries`
        // while it lays the tiles out, and a favourites edit is a `&mut self`.
        if let Some(name) = favourite_toggle {
            self.toggle_favorite(&name);
        }

        // Previews for the tiles on screen and the next screenful, top first.
        // Only those: the whole library queued is three hundred spheres path
        // traced one after another in the background, the best part of an
        // hour of a core every time the shelf had nothing kept, for tiles
        // nobody had scrolled to.
        self.queued = wanted(&visible, in_view)
            .rev()
            .filter(|&index| !self.sharp(self.keys[index]))
            .collect();
        self.render_queued(ui.ctx());

        action
    }

    /// The library's dialogs, drawn over everything: New material, details,
    /// and the question before a delete.
    fn dialogs(&mut self, ctx: &egui::Context, selected: Option<&crate::library_dialogs::Selected>) -> Option<LibraryAction> {
        let categories = library::categories(&self.entries);
        if let Some(made) = self.new_material.show(ctx, selected, &categories) {
            return Some(LibraryAction::Create(Box::new(made)));
        }
        if let Some(details) = self.details.show(ctx, &categories) {
            return Some(LibraryAction::EditDetails(details));
        }
        self.confirm_delete.show(ctx).map(LibraryAction::Delete)
    }

    /// Whether the tile of `key` has a preview as large as it is drawn. One
    /// smaller is shown until the larger is ready: made before the window
    /// knew it was on a Retina screen, or for a tile drawn smaller, it stayed
    /// for the session, a sphere of 96 pixels spread over 228.
    fn sharp(&self, key: u64) -> bool {
        self.thumbnails.get(&key).is_some_and(|texture| texture.size()[0] >= self.tile_pixels)
    }

    /// Upload finished previews and start at most one background job.
    fn render_queued(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.preview_job {
            if !job.is_finished() {
                return;
            }
            if let Some(Ok(Some((key, image)))) = self.preview_job.take().map(|job| job.join()) {
                let handle = ctx.load_texture(
                    format!("preview_{key:x}"),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                self.thumbnails.insert(key, handle);
            }
        }
        let Some(device) = self.device.clone() else {
            return;
        };
        let pixels = self.tile_pixels;
        let settings = tile_settings(pixels);
        let started = std::time::Instant::now();
        // Tiles read from disk are drawn the next frame, so ask for one.
        let mut loaded = false;

        while let Some(index) = self.queued.pop() {
            let (Some(entry), Some(&key)) = (self.entries.get(index), self.keys.get(index)) else {
                continue;
            };
            if self.sharp(key) {
                continue;
            }
            // Kept from an earlier launch: a file read, not a render.
            if let Some(image) = self.disk.load(crate::thumbnail::sized_key(key, pixels)) {
                let handle = ctx.load_texture(format!("preview_{key:x}"), image, egui::TextureOptions::LINEAR);
                self.thumbnails.insert(key, handle);
                loaded = true;
                if started.elapsed() > DISK_BUDGET {
                    break;
                }
                continue;
            }
            let graph = entry.graph.clone();
            let pictures = entry.textures.clone();
            let environment = self.environment.clone();
            let disk = self.disk.clone();
            let repaint = ctx.clone();
            self.preview_job = std::thread::Builder::new()
                .name("tequila-thumbnail".into())
                .spawn(move || {
                    // Thumbnail parallelism must not consume the viewport's
                    // global worker pool. Reuse one dedicated CPU worker, at
                    // the ordinary priority: in Windows' background mode it
                    // was starved whenever anything kept the processor busy -
                    // a viewport rendering on the processor kept the shelf
                    // grey until it settled.
                    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
                    let pool = POOL.get_or_init(|| rayon::ThreadPoolBuilder::new()
                        .num_threads(1).thread_name(|_| "tequila-preview-cpu".into())
                        .build().ok()).as_ref()?;
                    let textures = picture_table(&pictures);
                    let Ok(thumbnail) = pool.install(|| {
                        render_thumbnail_textured(&device, &graph, textures, environment, settings)
                    }) else {
                        repaint.request_repaint();
                        return None;
                    };

                    let rgba = crate::thumbnail::develop_rgba(&thumbnail);
                    disk.store(crate::thumbnail::sized_key(key, pixels), thumbnail.width, thumbnail.height, &rgba);
                    let image = egui::ColorImage::from_rgba_unmultiplied([thumbnail.width, thumbnail.height], &rgba);
                    repaint.request_repaint();
                    Some((key, image))
                })
                .ok();
            break;
        }
        if loaded {
            ctx.request_repaint();
        }
    }
}

/// The entries to preview: those on screen and as many again below them, so
/// a tile is usually ready by the time it is scrolled to.
fn wanted(visible: &[usize], in_view: std::ops::Range<usize>) -> impl DoubleEndedIterator<Item = usize> + '_ {
    let end = (in_view.end + in_view.len()).min(visible.len());
    visible[in_view.start.min(end)..end].iter().copied()
}

/// Where the favourite star sits on a tile.
fn star_rect(tile: egui::Rect, size: f32) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(tile.right() - 18.0, tile.top() + 2.0),
        egui::Vec2::splat(16.0),
    )
    .intersect(egui::Rect::from_min_size(tile.min, egui::Vec2::splat(size)))
}

/// The two marks a thumbnail can carry: favourite, and already in this project.
///
/// Drawn on the thumbnail rather than beside it. A 64 px tile has no spare
/// width, and a badge that pushes the name out of the way costs more than it
/// conveys — the name is what people are scanning for.
fn thumbnail_badges(ui: &egui::Ui, rect: egui::Rect, starred: bool, used: bool) {
    let painter = ui.painter();
    let hovered = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| rect.expand(4.0).contains(p));

    // The star shows always when set, and on hover when not — so the grid is
    // not a field of empty outlines, and the control is still discoverable by
    // moving the pointer over a tile.
    if starred || hovered {
        let centre = egui::pos2(rect.right() - 10.0, rect.top() + 10.0);
        painter.circle_filled(centre, 8.0, egui::Color32::from_black_alpha(110));
        painter.add(egui::Shape::convex_polygon(
            star_points(centre, 6.0),
            if starred {
                crate::theme::p().accent
            } else {
                egui::Color32::from_white_alpha(90)
            },
            egui::Stroke::NONE,
        ));
    }

    // A dot for "this material is already in the project" — §9's shelf shows
    // the same set as a filter, and the dot is what makes the filter's answer
    // legible without switching to it.
    if used {
        painter.circle_filled(
            egui::pos2(rect.left() + 9.0, rect.bottom() - 9.0),
            3.5,
            crate::theme::p().success,
        );
    }
}

/// A five-pointed star, outer radius `r`.
fn star_points(centre: egui::Pos2, r: f32) -> Vec<egui::Pos2> {
    let mut points = Vec::with_capacity(10);
    for i in 0..10 {
        let angle = std::f32::consts::FRAC_PI_2 * -1.0 + i as f32 * std::f32::consts::PI / 5.0;
        let radius = if i % 2 == 0 { r } else { r * 0.42 };
        points.push(egui::pos2(
            centre.x + angle.cos() * radius,
            centre.y + angle.sin() * radius,
        ));
    }
    points
}

/// Where the user's own materials live.
///
/// Beside the rest of the application's per-user state rather than in the
/// installation directory: an installation is replaceable and often read-only,
/// and a library that vanished when the application was updated would be the
/// worst possible failure for the one part of it a person built themselves.
pub fn user_library_path() -> std::path::PathBuf {
    library::user_directory()
}

/// How many materials the Recent shelf keeps.
const RECENT: usize = 24;

/// Where the Favorites and Recent shelves are kept: beside the person's
/// library, so they outlive restarts and updates as the library does.
pub struct ShelfFiles {
    pub favorites: std::path::PathBuf,
    pub recent: std::path::PathBuf,
}

impl ShelfFiles {
    fn beside_library() -> ShelfFiles {
        let folder = user_library_path().parent().map(|p| p.to_path_buf()).unwrap_or_else(std::env::temp_dir);
        ShelfFiles { favorites: folder.join("favorites.txt"), recent: folder.join("recent-materials.txt") }
    }
}

/// Material names kept one a line; none when the file is not there yet.
fn read_names(file: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .map(|text| text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// A shelf that cannot be written is a convenience lost, not an error to
/// interrupt anybody with.
fn write_names<'a>(file: &std::path::Path, names: impl Iterator<Item = &'a String>) {
    let text: String = names.map(|n| format!("{n}\n")).collect();
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(file, text);
}

/// What is in a folder of presets, as one number: their names and when
/// each last changed. Different whenever one is added, removed or saved.
fn folder_signature(folder: &std::path::Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    if let Ok(listing) = std::fs::read_dir(folder) {
        let mut files: Vec<(String, std::time::SystemTime)> = listing
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH)))
            .collect();
        files.sort();
        files.hash(&mut hasher);
    }
    hasher.finish()
}

/// Open a folder in the system's file browser, making it first if it is
/// not there yet - an empty library is still a place to put presets.
pub(crate) fn open_folder(folder: &std::path::Path) {
    let _ = std::fs::create_dir_all(folder);
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(folder).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(folder).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(folder).spawn();
}

/// How many thumbnails fit across, given the width available.
///
/// Pulled out of the layout because this is where the bug lived, and a
/// three-line calculation buried inside a closure inside a scroll area inside a
/// panel is a bug nothing can test.
///
/// The clamps are the fix. Without them the count follows `available_width`
/// without limit, and in a resizable side panel that is a loop: egui's `Grid`
/// remembers its column layout between frames, so a first frame measured
/// against an unconstrained width lays out a hundred columns, the panel grows
/// to fit them, and the viewport is squeezed out of the window.
fn columns_for(available: f32, thumbnail: f32) -> usize {
    let usable = available.min(MAX_WIDTH) - 8.0;
    let each = (thumbnail + 12.0).max(1.0);
    ((usable / each).floor() as usize).clamp(1, 8)
}

/// A key over the material's values, so a preview is reused for identical
/// materials and regenerated when one changes (§9).
pub fn content_key(material: &Material) -> u64 {
    use tequila_materials::material_type::Param;
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for param in Param::ALL {
        if let Some(value) = param.read(material) {
            format!("{value:?}").hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// The same over a whole graph: its surface values, and its patterns when it
/// has any, so a preset whose wood grain changes is previewed again.
pub fn graph_key(graph: &tequila_materials::graph::MaterialGraph) -> u64 {
    let surface = content_key(graph.material());
    if graph.len() > 1 {
        surface ^ graph.content_hash().rotate_left(17)
    } else {
        surface
    }
}

/// A preset's pictures, read for its tile: one made from a texture pack is
/// shown with its own maps rather than their fallbacks.
pub(crate) fn picture_table(
    pictures: &[(tequila_materials::graph::TextureId, tequila_scene::textures::TextureSource)],
) -> Arc<tequila_render_core::shade::TextureTable> {
    let mut textures = tequila_render_core::shade::TextureTable::new();
    for (id, source) in pictures {
        if let Some(texture) = tequila_scene::snapshot::load_texture(source) {
            textures.insert(*id, Arc::new(texture));
        }
    }
    Arc::new(textures)
}

/// A tile's key: its graph's, and the files of its pictures. Two presets
/// made from two texture packs have the same graph — the same maps into the
/// same parameters — and differ only in which files those are.
///
/// A file is its name and its size, not where it is. Thumbnails are kept
/// from one version to the next, and every version is installed in a folder
/// of its own, so a key with the whole path in it changed with every update
/// and the textured presets were rendered again each time. The size is what
/// tells a picture saved over in place from the one it replaced.
pub fn entry_key(entry: &LibraryEntry) -> u64 {
    use std::hash::{Hash, Hasher};
    let key = graph_key(&entry.graph);
    if entry.textures.is_empty() {
        return key;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (id, source) in &entry.textures {
        id.0.hash(&mut hasher);
        source.path.file_name().hash(&mut hasher);
        std::fs::metadata(&source.path).map(|file| file.len()).ok().hash(&mut hasher);
    }
    key ^ hasher.finish().rotate_left(29)
}

/// What a library tile is rendered with, at `density` pixels a point
/// (`thumbnail::density`): for the tests that judge the shelf's look.
#[cfg(test)]
pub(crate) fn thumbnail_settings_at(density: usize) -> PreviewSettings {
    tile_settings(96 * density.max(1))
}

/// What a library tile `pixels` across is rendered with.
fn tile_settings(pixels: usize) -> PreviewSettings {
    PreviewSettings { size: pixels, samples: 40, ..PreviewSettings::default() }
}

/// The pixels a tile `points` across is rendered at: what it covers on the
/// screen, so it is never enlarged. The tiles share the panel's width out,
/// 109 points each at the usual three across, 114 on a Mac; rendered at 96
/// and 192 they were spread over 109 and 228 pixels and came out soft. In
/// steps of 48, so a slider dragged across the range renders a few sizes, not
/// one for each pixel; at least 96, the size of the kept thumbnails, and at
/// most 384.
fn tile_pixels(points: f32, pixels_per_point: f32) -> usize {
    let covered = (points * pixels_per_point.max(0.5)).ceil().max(1.0) as usize;
    (covered.div_ceil(48) * 48).clamp(96, 384)
}

/// Where the bundled library lives.
///
/// Searched upwards from the executable rather than at a fixed depth, because
/// the depth differs by how the binary was produced: beside it once installed,
/// two levels up when run from `target/release`, three when run as a test
/// binary from `target/release/deps`. Guessing a depth gets one of those right
/// and leaves the browser silently empty in the others.
pub(crate) fn standard_library_path() -> std::path::PathBuf {
    library::standard_directory()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every preset in the standard library on one sheet, rendered as the
    /// thumbnails are but larger and cleaner, for judging the library as a
    /// whole: `TEQUILA_SHEET=out.png cargo test -p tequila-app --release
    /// library_contact_sheet -- --ignored --nocapture`. Prints which preset is
    /// which, row by row. `TEQUILA_SHEET_FILTER` keeps the names containing
    /// it; `TEQUILA_SHEET_DIR` reads another folder of presets.
    #[test]
    #[ignore]
    fn library_contact_sheet() {
        use rayon::prelude::*;
        let out = std::env::var("TEQUILA_SHEET").unwrap_or_else(|_| "library_sheet.png".into());
        let px: usize = std::env::var("TEQUILA_SHEET_PX").ok().and_then(|s| s.parse().ok()).unwrap_or(160);
        let samples: u32 = std::env::var("TEQUILA_SHEET_SPP").ok().and_then(|s| s.parse().ok()).unwrap_or(96);
        let filter = std::env::var("TEQUILA_SHEET_FILTER").ok();
        let folder = std::env::var("TEQUILA_SHEET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| standard_library_path());
        let (mut entries, errors) = library::read_directory(&folder);
        assert!(errors.is_empty(), "{errors:?}");
        // Several filters apart by `|` keep what matches any of them.
        entries.retain(|e| filter.as_deref().is_none_or(|f| f.split('|').any(|f| e.name.to_lowercase().contains(&f.to_lowercase()))));
        entries.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
        let device = Device::shared().expect("embree");
        let environment = Arc::new(tequila_scene::demo::preview_environment());
        let tiles: Vec<Vec<u8>> = entries
            .par_iter()
            .map(|entry| {
                let settings = PreviewSettings { size: px, samples, ..PreviewSettings::default() };
                let thumbnail = render_thumbnail_textured(&device, &entry.graph, picture_table(&entry.textures), environment.clone(), settings).expect("renders");
                // TEQUILA_SHEET_RAW: as rendered, without the denoiser.
                let rgba: Vec<u8> = if std::env::var_os("TEQUILA_SHEET_RAW").is_some() {
                    (0..thumbnail.width * thumbnail.height)
                        .flat_map(|i| {
                            let [r, g, b] = tequila_color::DisplayTransform::Srgb.apply_u8(thumbnail.pixel(i % thumbnail.width, i / thumbnail.width));
                            [r, g, b, 255]
                        })
                        .collect()
                } else {
                    crate::thumbnail::develop_rgba(&thumbnail)
                };
                // Transmissive materials render at twice the size: take every other pixel.
                let step = thumbnail.width / px;
                let mut tile = Vec::with_capacity(px * px * 3);
                for y in 0..px {
                    for x in 0..px {
                        let i = ((y * step) * thumbnail.width + x * step) * 4;
                        tile.extend_from_slice(&rgba[i..i + 3]);
                    }
                }
                tile
            })
            .collect();
        let columns = 10.min(tiles.len().max(1));
        let rows = tiles.len().div_ceil(columns);
        let gap = 4;
        let (w, h) = (columns * (px + gap) + gap, rows * (px + gap) + gap);
        let mut sheet = vec![30u8; w * h * 3];
        for (i, tile) in tiles.iter().enumerate() {
            let (cx, cy) = (gap + (i % columns) * (px + gap), gap + (i / columns) * (px + gap));
            for y in 0..px {
                let row = ((cy + y) * w + cx) * 3;
                sheet[row..row + px * 3].copy_from_slice(&tile[y * px * 3..(y + 1) * px * 3]);
            }
        }
        for (r, chunk) in entries.chunks(columns).enumerate() {
            println!("row {r}: {}", chunk.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(" | "));
        }
        let file = std::fs::File::create(&out).expect("writable");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&sheet).unwrap();
        println!("wrote {out}");
    }

    #[test]
    fn background_preview_keeps_its_material_key_when_entries_change() {
        let mut tab = tab();
        let context = egui::Context::default();
        let expected = graph_key(&tab.entries[0].graph);
        tab.queued.push(0);
        tab.render_queued(&context);
        assert!(tab.preview_job.is_some(), "preview should run on a worker");
        tab.entries.reverse();
        tab.keys.reverse();
        tab.queued.clear();
        // Generous: the preview shares the thread pool with every render
        // test in this binary, and timed out at 30 s under that load while
        // what this checks is which key it lands under, not how fast.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        while !tab.preview_job.as_ref().unwrap().is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "preview worker timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        tab.render_queued(&context);
        assert_eq!(tab.thumbnails.get(&expected).unwrap().size(), [96, 96]);
    }

    /// The browser with its three dialogs open - New material, details,
    /// the question before a delete - lays out frame after frame, each
    /// dialog on screen and inside it, and nothing is acted on without a
    /// click.
    #[test]
    fn the_library_and_its_dialogs_lay_out() {
        let mut tab = tab();
        // No thumbnails: their one shared worker belongs to the tests that
        // time it, and a layout test starting renders on it starved them.
        tab.device = None;
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let selected = crate::library_dialogs::Selected {
            name: "Paint".into(),
            material_type: tequila_materials::material_type::MaterialType::Paint,
            parts: 2,
        };
        let entry = tab.entries.iter().find(|e| e.name == "ABS White").expect("ships").clone();
        tab.new_material(Some(&selected));
        tab.details.open(&entry);
        tab.confirm_delete.ask(&entry);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
        for _ in 0..4 {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
                egui::SidePanel::left("library").exact_width(PANEL_WIDTH).show(ctx, |ui| {
                    let action = tab.show(ui, &BTreeSet::new(), Some(&selected));
                    assert!(action.is_none(), "{action:?} without a click");
                });
            });
        }
        for title in ["New material", "Material details", "Delete material?"] {
            let rect = ctx.memory(|m| m.area_rect(egui::Id::new(title))).unwrap_or_else(|| panic!("`{title}` is not on screen"));
            assert!(screen.contains_rect(rect), "`{title}` runs off the screen: {rect:?}");
            assert!(rect.width() > 200.0, "`{title}` is squeezed: {rect:?}");
        }
    }

    /// Only the tiles on screen, and the next screenful, wait for a preview
    /// — top first — and scrolling down moves the wait with it. The whole
    /// library queued was three hundred spheres path traced in the
    /// background whenever the shelf had nothing kept.
    #[test]
    fn only_tiles_on_screen_are_rendered() {
        let mut tab = tab();
        // Nothing renders, so what is waiting stays to be looked at.
        tab.device = None;
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 700.0));
        let frame = |tab: &mut LibraryTab, scroll: f32| {
            let mut input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            if scroll != 0.0 {
                input.events.push(egui::Event::PointerMoved(egui::pos2(100.0, 300.0)));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, scroll),
                    modifiers: Default::default(),
                });
            }
            let _ = ctx.run(input, |ctx| {
                egui::SidePanel::left("library").exact_width(PANEL_WIDTH).show(ctx, |ui| {
                    tab.show(ui, &BTreeSet::new(), None);
                });
            });
        };
        for _ in 0..3 {
            frame(&mut tab, 0.0);
        }
        let everything = tab.visible(&BTreeSet::new());
        assert!(everything.len() > 200, "the library did not load");
        let waiting: Vec<usize> = tab.queued.iter().rev().copied().collect();
        assert!(!waiting.is_empty(), "nothing on screen was queued");
        assert!(
            waiting.len() < everything.len() / 4,
            "{} of {} tiles queued with a screenful in view",
            waiting.len(),
            everything.len()
        );
        assert_eq!(waiting, everything[..waiting.len()], "not the top of the list, in order");

        // Scrolled well down, the wait is for the tiles there.
        for _ in 0..6 {
            frame(&mut tab, -2000.0);
        }
        frame(&mut tab, 0.0);
        let first = *tab.queued.last().expect("tiles in view are queued");
        let at = everything.iter().position(|&i| i == first).unwrap();
        assert!(at > waiting.len(), "after scrolling, the queue still starts at tile {at}");
    }

    #[test]
    fn the_next_screenful_is_wanted_too() {
        let visible: Vec<usize> = (100..200).collect();
        assert_eq!(wanted(&visible, 10..20).collect::<Vec<_>>(), (110..130).collect::<Vec<_>>());
        assert_eq!(wanted(&visible, 90..100).collect::<Vec<_>>(), (190..200).collect::<Vec<_>>());
        assert_eq!(wanted(&visible, 0..0).count(), 0);
        assert_eq!(wanted(&[], 0..0).count(), 0);
    }

    fn tab() -> LibraryTab {
        tab_keeping_in(&scratch_cache("tab"))
    }

    fn tab_keeping_in(folder: &std::path::Path) -> LibraryTab {
        LibraryTab::with_cache(
            Arc::new(tequila_scene::demo::preview_environment()),
            Arc::new(crate::thumbnail::DiskCache::in_folder(folder, "this-build")),
        )
    }

    /// A folder of its own for each test, emptied first.
    fn scratch_cache(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tequila-library-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A thumbnail rendered in one launch is shown by the next without
    /// rendering it again. The whole library is path traced and denoised to
    /// make its tiles, and doing that on every start kept the shelf grey for
    /// the first minute of every session.
    #[test]
    fn a_thumbnail_is_rendered_once_not_every_launch() {
        let folder = scratch_cache("relaunch");
        let context = egui::Context::default();
        let mut first = tab_keeping_in(&folder);
        let key = graph_key(&first.entries[0].graph);
        first.queued.push(0);
        first.render_queued(&context);
        // A guard against a worker that never finishes, not a measure of
        // speed: alone it takes seconds, and beside the rest of the suite,
        // sharing the processor with hundreds of tests, over a minute.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        while !first.preview_job.as_ref().expect("the first launch renders it").is_finished() {
            assert!(std::time::Instant::now() < deadline, "preview worker timed out");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        first.render_queued(&context);
        assert!(first.thumbnails.contains_key(&key));
        drop(first);

        let mut next = tab_keeping_in(&folder);
        next.queued.push(0);
        next.render_queued(&context);
        assert!(next.preview_job.is_none(), "the next launch rendered the thumbnail again");
        assert_eq!(
            next.thumbnails.get(&key).expect("the next launch shows it at once").size(),
            [96, 96]
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A tile is rendered at the pixels it covers, on any screen.
    #[test]
    fn a_tile_is_rendered_at_the_size_it_is_drawn() {
        assert_eq!(tile_pixels(96.0, 1.0), 96);
        assert_eq!(tile_pixels(109.0, 1.0), 144);
        assert_eq!(tile_pixels(96.0, 2.0), 192);
        assert_eq!(tile_pixels(114.0, 2.0), 240);
        assert_eq!(tile_pixels(40.0, 1.0), 96, "a small tile keeps the kept thumbnails");
        assert_eq!(tile_pixels(320.0, 2.0), 384, "one tile across the panel is bounded");
        // The thumbnails kept at 96 and 192 keep their keys.
        assert_eq!(crate::thumbnail::sized_key(7, 96), 7);
        assert_eq!(crate::thumbnail::sized_key(7, 192), crate::thumbnail::dense_key(7, 2));
        let others = [144, 240, 288, 336, 384].map(|pixels| crate::thumbnail::sized_key(7, pixels));
        assert!(others.iter().all(|&key| key != 7 && key != crate::thumbnail::dense_key(7, 2)));
    }

    /// A preview smaller than its tile is rendered again at the tile's size,
    /// and shown until then: on a Mac the shelf kept spheres of 96 pixels
    /// drawn over 228 for a whole session.
    #[test]
    fn a_preview_smaller_than_its_tile_is_rendered_again() {
        let folder = scratch_cache("larger");
        let context = egui::Context::default();
        let mut tab = tab_keeping_in(&folder);
        let key = graph_key(&tab.entries[0].graph);
        let small = context.load_texture("small", egui::ColorImage::new([96, 96], egui::Color32::GRAY), egui::TextureOptions::LINEAR);
        tab.thumbnails.insert(key, small);
        tab.queued.push(0);
        tab.render_queued(&context);
        assert!(tab.preview_job.is_none(), "a tile of 96 pixels was rendered again");

        tab.tile_pixels = 144;
        tab.queued.push(0);
        tab.render_queued(&context);
        assert_eq!(tab.thumbnails.get(&key).unwrap().size(), [96, 96], "the smaller one is shown meanwhile");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        while !tab.preview_job.as_ref().expect("a larger tile renders its preview again").is_finished() {
            assert!(std::time::Instant::now() < deadline, "preview worker timed out");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        tab.render_queued(&context);
        assert_eq!(tab.thumbnails.get(&key).unwrap().size(), [144, 144]);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The bundled library has to be found from a development checkout, or the
    /// browser is empty every time anyone runs the app from source.
    #[test]
    fn the_standard_library_is_found() {
        let tab = tab();
        assert!(
            tab.entries().len() >= 50,
            "found {} presets at {:?}",
            tab.entries().len(),
            standard_library_path()
        );
        assert!(tab.problems.is_empty(), "{:?}", tab.problems);
    }

    /// The column count must not follow the width it is given without limit.
    ///
    /// This is a regression test for a layout feedback loop that made the app
    /// unusable. egui's `Grid` remembers its column layout between frames, and
    /// a resizable side panel grows to fit its contents. A first frame measured
    /// against an unconstrained width laid out over a hundred columns; the
    /// panel grew to hold them; the viewport was squeezed to nothing and the
    /// scene could not be seen at all.
    ///
    /// Tested against the calculation rather than against a rendered layout,
    /// because the rendered version cannot distinguish it: separators and
    /// wrapped rows legitimately fill whatever container they are in, so
    /// measuring the panel measures them rather than the grid.
    #[test]
    fn the_column_count_does_not_follow_an_unbounded_width() {
        // What an unconstrained first frame offers.
        assert!(
            columns_for(9984.0, 64.0) <= 8,
            "an unbounded width gave {} columns",
            columns_for(9984.0, 64.0)
        );
        assert!(columns_for(f32::INFINITY, 64.0) <= 8);

        // Realistic panel widths still fit a sensible number.
        assert_eq!(columns_for(240.0, 64.0), 3);
        assert_eq!(columns_for(180.0, 64.0), 2);

        // A narrow panel, or an enormous thumbnail, still shows one.
        assert_eq!(columns_for(20.0, 64.0), 1);
        assert_eq!(columns_for(240.0, 1000.0), 1);
        assert_eq!(columns_for(0.0, 0.0), 1);
    }

    /// Filtering composes: a shelf and a search narrow together rather than one
    /// overriding the other.
    #[test]
    fn shelves_and_search_narrow_together() {
        let mut tab = tab();
        let everything = tab.visible(&BTreeSet::new()).len();

        tab.query = "metal".into();
        let searched = tab.visible(&BTreeSet::new()).len();
        assert!(searched > 0 && searched < everything);

        tab.shelf = Shelf::Favorites;
        assert!(
            tab.visible(&BTreeSet::new()).is_empty(),
            "an empty favorites shelf should show nothing regardless of search"
        );

        let name = tab.entries()[0].name.clone();
        tab.favorites.insert(name.clone());
        tab.query.clear();
        let favorites = tab.visible(&BTreeSet::new());
        assert_eq!(favorites.len(), 1);
        assert_eq!(tab.entries()[favorites[0]].name, name);
    }

    /// Favorites and Recent are what they were after a restart. They were
    /// in memory only, and a community member lost every favorite updating
    /// the Mac application.
    #[test]
    fn favorites_and_recent_outlive_a_restart() {
        let folder = std::env::temp_dir().join(format!("tequila-shelves-{}", std::process::id()));
        let files = || ShelfFiles { favorites: folder.join("favorites.txt"), recent: folder.join("recent-materials.txt") };
        let mut before = tab();
        before.keep_shelves(files());
        let (a, b) = (before.entries()[0].name.clone(), before.entries()[1].name.clone());
        before.toggle_favorite(&a);
        before.toggle_favorite(&b);
        before.toggle_favorite(&a);
        before.note_used(&a);
        before.note_used(&b);

        let mut after = tab();
        after.keep_shelves(files());
        assert_eq!(after.favorites, BTreeSet::from([b.clone()]));
        assert_eq!(after.recent, vec![b, a]);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Recently-used keeps the most recent first, without duplicates.
    #[test]
    fn recent_is_ordered_and_deduplicated() {
        let mut tab = tab();
        tab.note_used("A");
        tab.note_used("B");
        tab.note_used("A");
        assert_eq!(tab.recent, vec!["A".to_string(), "B".to_string()]);

        for i in 0..40 {
            tab.note_used(&format!("m{i}"));
        }
        assert!(tab.recent.len() <= 24, "recent grew without bound");
    }

    /// Two materials with identical parameters share a preview; changing one
    /// gives it a new key, which is what makes the cache correct rather than
    /// merely fast.
    #[test]
    fn the_preview_key_follows_the_material() {
        use tequila_materials::material_type::MaterialType;
        let a = MaterialType::Plastic.defaults();
        let b = MaterialType::Plastic.defaults();
        assert_eq!(content_key(&a), content_key(&b));

        let changed = Material {
            specular_roughness: a.specular_roughness + 0.1,
            ..a.clone()
        };
        assert_ne!(
            content_key(&a),
            content_key(&changed),
            "a changed material reused its old preview"
        );
    }

    /// The "in project" shelf reflects the document, not the library.
    #[test]
    fn the_in_project_shelf_follows_the_document() {
        let mut tab = tab();
        tab.shelf = Shelf::InProject;
        assert!(tab.visible(&BTreeSet::new()).is_empty());

        let name = tab.entries()[3].name.clone();
        let in_project: BTreeSet<String> = [name.clone()].into_iter().collect();
        let visible = tab.visible(&in_project);
        assert_eq!(visible.len(), 1);
        assert_eq!(tab.entries()[visible[0]].name, name);
    }
}

/// Show a file in the platform's file manager.
///
/// Best effort and deliberately silent on failure: this is a convenience, and
/// an error dialog because a file manager is missing would be worse than the
/// button quietly doing nothing on a machine that has none.
pub(crate) fn reveal(path: &str) {
    if path.is_empty() {
        return;
    }
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let folder = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let _ = std::process::Command::new("xdg-open").arg(folder).spawn();
    }
}
