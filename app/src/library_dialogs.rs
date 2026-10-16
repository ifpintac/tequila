//! The library's own dialogs: making a new material, changing the details
//! of one of your own, and making sure before one is deleted.
//!
//! Making a material is the library's job, not only the Material tab's.
//! The place people look for materials is the place they expect to add
//! one, and "add" there means four quite different things - start from a
//! type, keep what is on the part I tuned, turn a downloaded texture pack
//! into a preset, bring in a file somebody sent - so the dialog asks which
//! first, and then only what that way needs.

use eframe::egui;
use tequila_materials::material_type::MaterialType;
use std::path::PathBuf;

/// Where a new material starts from.
#[derive(Clone, Debug, PartialEq)]
pub enum Start {
    /// A material type's own defaults, in a colour (sRGB, as a swatch shows
    /// it) and a roughness.
    Blank { material_type: MaterialType, colour: [u8; 3], roughness: f32 },
    /// The material on the selected part, as it is now.
    Selection,
    /// A downloaded texture pack, a folder or a ZIP of maps, one repeat
    /// `size` metres across.
    TexturePack { path: PathBuf, size: f32 },
    /// A material file: a package, or a preset's `.json`.
    File(PathBuf),
}

/// What the dialog asked for.
#[derive(Clone, Debug)]
pub struct NewMaterial {
    pub start: Start,
    /// Empty for a file: it keeps its own.
    pub name: String,
    pub category: String,
    pub tags: Vec<String>,
    /// Put it on the selected parts once it is made.
    pub apply: bool,
}

/// The selected part's material, as the dialog describes it.
pub struct Selected {
    pub name: String,
    pub material_type: MaterialType,
    pub parts: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StartKind {
    Blank,
    Selection,
    Pack,
    File,
}

pub struct NewMaterialDialog {
    open: bool,
    kind: StartKind,
    material_type: MaterialType,
    colour: [u8; 3],
    roughness: f32,
    pack: Option<PathBuf>,
    pack_size_cm: f32,
    file: Option<PathBuf>,
    name: String,
    category: String,
    tags: String,
    apply: bool,
    /// Typed by the person rather than suggested: a suggestion follows
    /// what they pick; their own words are not overwritten.
    named_by_hand: bool,
}

impl Default for NewMaterialDialog {
    fn default() -> Self {
        NewMaterialDialog {
            open: false,
            kind: StartKind::Blank,
            material_type: MaterialType::Plastic,
            colour: [200, 200, 196],
            roughness: 0.3,
            pack: None,
            pack_size_cm: 30.0,
            file: None,
            name: String::new(),
            category: String::new(),
            tags: String::new(),
            apply: false,
            named_by_hand: false,
        }
    }
}

/// Words split at commas, trimmed, the empty ones dropped.
pub fn tags_from(text: &str) -> Vec<String> {
    text.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
}

impl NewMaterialDialog {
    /// Open, starting from the selection's material if there is one and
    /// from a blank plastic if not; or straight from a file or pack that is
    /// already chosen.
    pub fn open(&mut self, selected: Option<&Selected>) {
        *self = NewMaterialDialog { open: true, ..NewMaterialDialog::default() };
        if let Some(selected) = selected {
            self.kind = StartKind::Selection;
            self.apply = false;
            self.name = selected.name.clone();
            self.category = suggest_category(selected.material_type);
        } else {
            self.suggest();
        }
    }

    /// A name and category for what is picked, until the person types one.
    fn suggest(&mut self) {
        if self.named_by_hand {
            return;
        }
        match self.kind {
            StartKind::Blank => {
                self.name = format!("My {}", self.material_type.label());
                self.category = suggest_category(self.material_type);
            }
            StartKind::Pack => {
                if let Some(path) = &self.pack {
                    self.name = pack_name(path);
                    self.category = "Textured".into();
                }
            }
            StartKind::File | StartKind::Selection => {}
        }
    }

    /// Draw it, if open. `Some` when the person asked for a material.
    pub fn show(&mut self, ctx: &egui::Context, selected: Option<&Selected>, categories: &[String]) -> Option<NewMaterial> {
        if !self.open {
            return None;
        }
        let mut open = self.open;
        let mut made = None;
        egui::Window::new("New material")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(360.0);
                ui.label(egui::RichText::new("Start from").strong());
                let before = self.kind;
                let mut kinds = vec![(StartKind::Blank, "Blank")];
                if selected.is_some() {
                    kinds.push((StartKind::Selection, "Selected part"));
                }
                kinds.push((StartKind::Pack, "Texture pack"));
                kinds.push((StartKind::File, "File"));
                if selected.is_none() && self.kind == StartKind::Selection {
                    self.kind = StartKind::Blank;
                }
                crate::theme::segmented(ui, &mut self.kind, &kinds);
                if self.kind != before {
                    self.suggest();
                }
                ui.add_space(6.0);
                let ready = match self.kind {
                    StartKind::Blank => {
                        egui::Grid::new("new-material-blank").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                            ui.label("Type");
                            let before = self.material_type;
                            egui::ComboBox::from_id_salt("new-material-type")
                                .selected_text(self.material_type.label())
                                .width(200.0)
                                .height(320.0)
                                .show_ui(ui, |ui| {
                                    for ty in MaterialType::ALL {
                                        ui.selectable_value(&mut self.material_type, ty, ty.label());
                                    }
                                });
                            if self.material_type != before {
                                self.suggest();
                            }
                            ui.end_row();
                            ui.label("Colour");
                            ui.color_edit_button_srgb(&mut self.colour);
                            ui.end_row();
                            ui.label("Roughness");
                            ui.add(crate::field::Slider::new(&mut self.roughness, 0.0..=1.0));
                            ui.end_row();
                        });
                        hint(ui, "A starting point: the type's own physics in your colour. Its patterns, maps and every other value are yours to add in the Material tab and the node editor.");
                        true
                    }
                    StartKind::Selection => {
                        if let Some(selected) = selected {
                            hint(
                                ui,
                                &format!(
                                    "`{}` ({}), as it is on the selected part{} now - patterns, image maps and all. The maps are copied into your library, so it keeps working when their folder is tidied away.",
                                    selected.name,
                                    selected.material_type.label(),
                                    if selected.parts == 1 { "" } else { "s" }
                                ),
                            );
                        }
                        selected.is_some()
                    }
                    StartKind::Pack => {
                        ui.horizontal(|ui| {
                            if ui.button("Choose folder…").clicked() {
                                if let Some(path) = crate::texture_pack::pick(true) {
                                    self.pack = Some(path);
                                    self.suggest();
                                }
                            }
                            if ui.button("Choose ZIP…").clicked() {
                                if let Some(path) = crate::texture_pack::pick(false) {
                                    self.pack = Some(path);
                                    self.suggest();
                                }
                            }
                        });
                        if let Some(path) = &self.pack {
                            ui.label(egui::RichText::new(path.display().to_string()).small().monospace());
                        }
                        ui.horizontal(|ui| {
                            ui.label("One repeat covers");
                            ui.add(egui::DragValue::new(&mut self.pack_size_cm).range(0.5..=500.0).suffix(" cm").speed(0.5));
                        });
                        hint(ui, "A PBR pack from ambientCG, Poly Haven and the like: its colour, roughness, metalness, normal and height maps become one material, the maps copied into your library.");
                        self.pack.is_some()
                    }
                    StartKind::File => {
                        if ui.button("Choose file…").clicked() {
                            if let Some(path) = pick_material_file() {
                                self.file = Some(path);
                            }
                        }
                        if let Some(path) = &self.file {
                            ui.label(egui::RichText::new(path.display().to_string()).small().monospace());
                        }
                        hint(ui, "A material somebody shared: a Tequila material package (.fsmat) or a preset's .json. It keeps its own name unless you give it another here.");
                        self.file.is_some()
                    }
                };
                ui.add_space(8.0);
                egui::Grid::new("new-material-details").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    let hint_text = if self.kind == StartKind::File { "as the file says" } else { "" };
                    if ui.add(egui::TextEdit::singleline(&mut self.name).hint_text(hint_text).desired_width(230.0)).changed() {
                        self.named_by_hand = true;
                    }
                    ui.end_row();
                    ui.label("Category");
                    ui.horizontal(|ui| {
                        let hint_text = if self.kind == StartKind::File { "as the file says" } else { "" };
                        ui.add(egui::TextEdit::singleline(&mut self.category).hint_text(hint_text).desired_width(196.0));
                        egui::ComboBox::from_id_salt("new-material-category").selected_text("").width(24.0).show_ui(ui, |ui| {
                            for category in categories {
                                if ui.selectable_label(&self.category == category, category).clicked() {
                                    self.category = category.clone();
                                }
                            }
                        });
                    });
                    ui.end_row();
                    ui.label("Tags");
                    ui.add(egui::TextEdit::singleline(&mut self.tags).hint_text("brushed, satin, housing").desired_width(230.0));
                    ui.end_row();
                });
                if selected.is_some() && self.kind != StartKind::Selection {
                    ui.checkbox(&mut self.apply, "Put it on the selected parts");
                }
                let named = self.kind == StartKind::File || (!self.name.trim().is_empty() && !self.category.trim().is_empty());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let create = ui.add_enabled(ready && named, egui::Button::new(egui::RichText::new("Add to my library").strong()));
                    if create.clicked() {
                        let start = match self.kind {
                            StartKind::Blank => Start::Blank { material_type: self.material_type, colour: self.colour, roughness: self.roughness },
                            StartKind::Selection => Start::Selection,
                            StartKind::Pack => Start::TexturePack { path: self.pack.clone().expect("ready"), size: self.pack_size_cm / 100.0 },
                            StartKind::File => Start::File(self.file.clone().expect("ready")),
                        };
                        made = Some(NewMaterial {
                            start,
                            name: self.name.trim().to_string(),
                            category: self.category.trim().to_string(),
                            tags: tags_from(&self.tags),
                            apply: self.apply && self.kind != StartKind::Selection,
                        });
                        self.open = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.open = false;
                    }
                });
            });
        if !open {
            self.open = false;
        }
        made
    }
}

/// Changing one of your own: its name, its category, its tags.
#[derive(Default)]
pub struct DetailsDialog {
    target: Option<String>,
    name: String,
    category: String,
    tags: String,
}

/// What the details dialog asked for: the entry, by its file, and its new
/// details.
#[derive(Clone, Debug)]
pub struct Details {
    pub path: String,
    pub name: String,
    pub category: String,
    pub tags: Vec<String>,
}

impl DetailsDialog {
    pub fn open(&mut self, entry: &tequila_format::library::LibraryEntry) {
        self.target = Some(entry.path.clone());
        self.name = entry.name.clone();
        self.category = entry.category.clone();
        self.tags = entry.tags.join(", ");
    }

    pub fn show(&mut self, ctx: &egui::Context, categories: &[String]) -> Option<Details> {
        let path = self.target.clone()?;
        let mut open = true;
        let mut done = None;
        egui::Window::new("Material details")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                egui::Grid::new("material-details").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(230.0));
                    ui.end_row();
                    ui.label("Category");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.category).desired_width(196.0));
                        egui::ComboBox::from_id_salt("details-category").selected_text("").width(24.0).show_ui(ui, |ui| {
                            for category in categories {
                                if ui.selectable_label(&self.category == category, category).clicked() {
                                    self.category = category.clone();
                                }
                            }
                        });
                    });
                    ui.end_row();
                    ui.label("Tags");
                    ui.add(egui::TextEdit::singleline(&mut self.tags).hint_text("words to find it by").desired_width(230.0));
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let ready = !self.name.trim().is_empty() && !self.category.trim().is_empty();
                    if ui.add_enabled(ready, egui::Button::new(egui::RichText::new("Save").strong())).clicked() {
                        done = Some(Details {
                            path: path.clone(),
                            name: self.name.trim().to_string(),
                            category: self.category.trim().to_string(),
                            tags: tags_from(&self.tags),
                        });
                    }
                    if ui.button("Cancel").clicked() {
                        self.target = None;
                    }
                });
            });
        if done.is_some() || !open {
            self.target = None;
        }
        done
    }
}

/// Asking before a material of your own is deleted: its file and its
/// pictures go, and there is no undo for a file.
#[derive(Default)]
pub struct ConfirmDelete {
    target: Option<(String, String)>,
}

impl ConfirmDelete {
    pub fn ask(&mut self, entry: &tequila_format::library::LibraryEntry) {
        self.target = Some((entry.path.clone(), entry.name.clone()));
    }

    /// `Some(path)` once the person has said yes.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<String> {
        let (path, name) = self.target.clone()?;
        let mut open = true;
        let mut confirmed = None;
        egui::Window::new("Delete material?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!("`{name}` leaves your library: its file and its copies of any pictures. Projects that use it keep their own copy of the material."));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let delete = egui::Button::new(egui::RichText::new("Delete").color(crate::theme::p().danger).strong());
                    if ui.add(delete).clicked() {
                        confirmed = Some(path.clone());
                    }
                    if ui.button("Keep it").clicked() {
                        self.target = None;
                    }
                });
            });
        if confirmed.is_some() || !open {
            self.target = None;
        }
        confirmed
    }
}

/// A category a type most likely belongs under.
pub fn suggest_category(material_type: MaterialType) -> String {
    use MaterialType::*;
    match material_type {
        Metal | Anisotropic => "Metal",
        Plastic | PlasticCloudy | PlasticTransparent => "Plastic",
        Paint | MetallicPaint => "Paint",
        Glass | GlassSolid | Dielectric => "Glass",
        Gem => "Gem",
        Liquid => "Liquid",
        Translucent => "Translucent",
        _ => "Other",
    }
    .to_string()
}

/// A pack's name, from its folder or file, without the resolution and
/// format its download was: "Leather037_1K-JPG" is Leather037, and Poly
/// Haven's "brown_leather_2k" is Brown Leather.
fn pack_name(path: &std::path::Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let resolution = |word: &str| {
        let lower = word.to_ascii_lowercase();
        let head = lower.split('-').next().unwrap_or("");
        (head.ends_with('k') && head.len() >= 2 && head[..head.len() - 1].chars().all(|c| c.is_ascii_digit()))
            || matches!(lower.as_str(), "jpg" | "png" | "exr" | "jpeg" | "tif")
    };
    let words: Vec<String> = stem
        .split('_')
        .filter(|w| !w.is_empty() && !resolution(w))
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect();
    if words.is_empty() {
        "Texture pack".into()
    } else {
        words.join(" ")
    }
}

/// Ask for a material file.
pub fn pick_material_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Import a material")
        .add_filter("Material", &["fsmat", "json", "zip"])
        .pick_file()
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().color(crate::theme::dim()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_split_at_commas_and_trimmed() {
        assert_eq!(tags_from(" brushed, ,satin ,"), vec!["brushed".to_string(), "satin".to_string()]);
    }

    #[test]
    fn a_packs_name_is_its_set_not_its_resolution() {
        assert_eq!(pack_name(std::path::Path::new("C:/Downloads/Leather037_1K-JPG.zip")), "Leather037");
        assert_eq!(pack_name(std::path::Path::new("C:/Downloads/brown_leather_2k")), "Brown Leather");
        assert_eq!(pack_name(std::path::Path::new("C:/Downloads/oak_veneer_01")), "Oak Veneer 01");
    }
}
