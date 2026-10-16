//! What the library's actions do: make a material, copy one, change its
//! details, replace it with what is on a part, export it, import one, and
//! put one on parts. The browser and the dialogs say what was asked; this
//! is where it happens, so a material dropped on the window, picked from a
//! menu or made in the dialog lands the same way.

use crate::library_dialogs::{NewMaterial, Selected, Start};
use crate::library_tab::{user_library_path, LibraryAction};
use tequila_format::library::{self, LibraryEntry, Source};
use tequila_materials::graph::MaterialGraph;
use tequila_materials::material_type::Param;
use std::path::{Path, PathBuf};

impl crate::TequilaApp {
    /// The material on the selected parts - the first one, and how many
    /// parts carry a material at all - for the dialogs that start from it.
    pub(crate) fn selected_material(&self) -> Option<(tequila_scene::tree::MaterialKey, Selected)> {
        let document = self.project.document();
        let mut first = None;
        let mut parts = 0;
        for node in self.project.selection.nodes() {
            for id in document.tree.descendants_and_self(node) {
                if let Some(key) = document.tree.get(id).and_then(|n| n.material) {
                    parts += 1;
                    first.get_or_insert(key);
                }
            }
        }
        let key = first?;
        let definition = document.materials.get(key)?;
        Some((key, Selected { name: definition.name.clone(), material_type: definition.material_type, parts }))
    }

    /// Do what the library browser asked.
    pub(crate) fn library_action(&mut self, action: LibraryAction) {
        let directory = user_library_path();
        match action {
            LibraryAction::ApplyToSelection(entry) => {
                // §3.3: double-click with a selection applies to it.
                let nodes: Vec<tequila_scene::tree::NodeId> = self.project.selection.nodes().collect();
                if !nodes.is_empty() {
                    self.apply_library_material(&entry, &nodes, false);
                }
            }
            LibraryAction::BeganDrag(entry) => self.dragging = Some(entry),
            LibraryAction::ImportTexturePack(folder) => self.pick_texture_pack(folder),
            LibraryAction::Create(new) => {
                let (name, apply) = (new.name.clone(), new.apply);
                match self.create_material(*new) {
                    Ok(path) => {
                        self.library.reload();
                        self.library.show_mine();
                        let made = self.library_entry_at(&path.display().to_string());
                        let shown = made.as_ref().map(|e| e.name.clone()).unwrap_or(name);
                        if let (true, Some(entry)) = (apply, made) {
                            let nodes: Vec<tequila_scene::tree::NodeId> = self.project.selection.nodes().collect();
                            self.apply_library_material(&entry, &nodes, false);
                        }
                        self.toasts.info(format!("Added `{shown}` to your library"));
                    }
                    Err(error) => self.toasts.warn(error),
                }
            }
            LibraryAction::Duplicate(path) => {
                let Some(entry) = self.library_entry_at(&path) else { return };
                let name = format!("{} copy", entry.name);
                match library::save_copy(&entry, &directory, &name) {
                    Ok(saved) => {
                        self.library.reload();
                        self.library.show_mine();
                        // Straight to naming it: a copy is a starting point.
                        self.library.edit(&saved.display().to_string());
                        self.toasts.info(format!("Copied `{}` into your library", entry.name));
                    }
                    Err(error) => self.toasts.warn(error.to_string()),
                }
            }
            LibraryAction::EditDetails(details) => {
                let Some(entry) = self.library_entry_at(&details.path) else { return };
                match library::edit_details(&entry, &details.name, &details.category, &details.tags) {
                    Ok(edited) => {
                        self.library.reload();
                        self.toasts.info(format!("Saved `{}`", edited.name));
                    }
                    Err(error) => self.toasts.warn(error.to_string()),
                }
            }
            LibraryAction::UpdateFromSelection(path) => {
                let Some(entry) = self.library_entry_at(&path) else { return };
                let Some((key, _)) = self.selected_material() else {
                    self.toasts.warn("Select a part whose material you tuned".to_string());
                    return;
                };
                let document = self.project.document();
                let Some(definition) = document.materials.get(key) else { return };
                let replacement = LibraryEntry::from_graph_with_pictures(
                    entry.name.clone(),
                    entry.category.clone(),
                    definition.material_type,
                    &definition.graph,
                    &document.textures,
                    Source::User,
                );
                match library::replace_material(&entry, &directory, &replacement) {
                    Ok(_) => {
                        self.library.reload();
                        self.toasts.info(format!("Updated `{}` from the selected part", entry.name));
                    }
                    Err(error) => self.toasts.warn(error.to_string()),
                }
            }
            LibraryAction::Export(path) => {
                let Some(entry) = self.library_entry_at(&path) else { return };
                let file = format!("{}.{}", library::file_stem(&entry.name), library::PACKAGE_EXTENSION);
                let Some(to) = rfd::FileDialog::new()
                    .set_title("Export material")
                    .set_file_name(file)
                    .add_filter("Tequila material", &[library::PACKAGE_EXTENSION])
                    .save_file()
                else {
                    return;
                };
                match library::export_package(&entry, &to) {
                    Ok(()) => self.toasts.info(format!("Exported `{}` to {}", entry.name, to.display())),
                    Err(error) => self.toasts.warn(error.to_string()),
                }
            }
            LibraryAction::ImportFile => {
                if let Some(path) = crate::library_dialogs::pick_material_file() {
                    self.import_material_file(&path);
                }
            }
            LibraryAction::OpenFolder => crate::library_tab::open_folder(&directory),
            LibraryAction::Delete(path) => {
                // Deleting a file is not undoable through the command bus -
                // the bus is the document's history and this is not in the
                // document - so the guards are that only the user's own
                // entries offer the action, that it was asked and answered,
                // and that `tequila-format` checks again rather than
                // trusting the caller.
                let Some(entry) = self.library_entry_at(&path) else { return };
                match library::delete_entry(&entry) {
                    Ok(()) => {
                        self.library.reload();
                        self.toasts.info(format!("Deleted `{}` from your library", entry.name));
                    }
                    Err(error) => self.toasts.warn(error.to_string()),
                }
            }
        }
    }

    /// A material file - a package or a preset - into the user's library,
    /// however it arrived: dropped, picked, or named by a script.
    pub(crate) fn import_material_file(&mut self, path: &Path) {
        match library::import_material_file(path, &user_library_path(), |_| {}) {
            Ok(saved) => {
                self.library.reload();
                self.library.show_mine();
                let name = self.library_entry_at(&saved.display().to_string()).map(|e| e.name).unwrap_or_default();
                self.toasts.info(format!("Added `{name}` to your library"));
            }
            Err(error) => self.toasts.warn(error.to_string()),
        }
    }

    fn library_entry_at(&self, path: &str) -> Option<LibraryEntry> {
        self.library.entries().iter().find(|e| e.path == path).cloned()
    }

    /// Make the material the dialog describes and save it into the user's
    /// library. The file it went to.
    fn create_material(&mut self, new: NewMaterial) -> Result<PathBuf, String> {
        let directory = user_library_path();
        let NewMaterial { start, name, category, tags, .. } = new;
        let saved = match start {
            Start::Blank { material_type, colour, roughness } => {
                let mut material = material_type.defaults();
                let exposed = material_type.exposed();
                if exposed.contains(&Param::BaseColor) {
                    material.base_color = from_srgb(colour);
                }
                if exposed.contains(&Param::SpecularRoughness) {
                    material.specular_roughness = roughness;
                }
                let graph = MaterialGraph::from_material(material);
                let mut entry = LibraryEntry::from_graph(name, category, material_type, &graph, Source::User);
                entry.tags = tags;
                library::save_entry(&directory, &entry)
            }
            Start::Selection => {
                let (key, _) = self.selected_material().ok_or("Select a part with a material first")?;
                let document = self.project.document();
                let definition = document.materials.get(key).ok_or("The selected part's material is gone")?;
                let mut entry = LibraryEntry::from_graph_with_pictures(name, category, definition.material_type, &definition.graph, &document.textures, Source::User);
                entry.tags = tags;
                library::save_entry(&directory, &entry)
            }
            Start::TexturePack { path, size } => {
                let entry = pack_entry(&path, size, name, category, tags)?;
                library::save_entry(&directory, &entry)
            }
            Start::File(path) => library::import_material_file(&path, &directory, |entry| {
                if !name.is_empty() {
                    entry.name = name;
                }
                if !category.is_empty() {
                    entry.category = category;
                }
                if !tags.is_empty() {
                    entry.tags = tags;
                }
            }),
        };
        saved.map_err(|e| e.to_string())
    }
}

/// A colour picked as a swatch shows it, in the rendering space.
fn from_srgb(colour: [u8; 3]) -> tequila_core::spectrum::Spectrum {
    let linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    tequila_core::spectrum::Spectrum::from_linear_srgb(tequila_core::math::vec3(linear(colour[0]), linear(colour[1]), linear(colour[2])))
}

/// A texture pack made into a library entry, without touching the open
/// project (`texture_set::library_entry`, which scripts use too).
pub(crate) fn pack_entry(path: &Path, size: f32, name: String, category: String, tags: Vec<String>) -> Result<LibraryEntry, String> {
    tequila_import::texture_set::library_entry(path, size, name, category, tags).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_materials::graph::NodeKind;

    /// A texture pack becomes a library entry with its maps, sized as asked,
    /// and the open project is not touched.
    #[test]
    fn a_texture_pack_becomes_a_library_entry() {
        let folder = std::env::temp_dir().join(format!("tequila-pack-entry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        for (name, rgb) in [("Planks012_1K-PNG_Color.png", [150u8, 100, 60]), ("Planks012_1K-PNG_Roughness.png", [180, 180, 180])] {
            let file = std::fs::File::create(folder.join(name)).unwrap();
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 4, 4);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let data: Vec<u8> = (0..16).flat_map(|_| rgb).collect();
            encoder.write_header().unwrap().write_image_data(&data).unwrap();
        }
        let entry = pack_entry(&folder, 0.3, "Planks".into(), "Wood".into(), vec!["oak".into()]).expect("makes an entry");
        assert_eq!(entry.textures.len(), 2, "{:?}", entry.textures);
        assert!(entry.textures.iter().all(|(_, s)| s.path.starts_with(&folder)));
        let mapped = entry.graph.nodes().any(|(_, n)| matches!(&n.kind, NodeKind::Mapping2d { mapping } if (mapping.scale.x - 0.3).abs() < 1e-6));
        assert!(mapped, "the repeat is not the size asked for");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_swatch_colour_is_converted_to_linear() {
        let white = from_srgb([255, 255, 255]);
        assert!((white.r - 1.0).abs() < 1e-3 && (white.g - 1.0).abs() < 1e-3);
        let mid = from_srgb([128, 128, 128]);
        assert!((mid.g - 0.2158).abs() < 0.01, "{mid:?}");
    }
}
