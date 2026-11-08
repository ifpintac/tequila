//! The Library's Studios section: studio presets to apply in one step,
//! grouped by family with their sizes side by side, the size nearest the
//! product marked; the scene's studio saved as one; presets installed from
//! a package, exported, deleted (`docs/studio-presets.md`).

use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use tequila_format::studio_preset::{self as presets, PresetFile, PresetSource};
use tequila_scene::preset::{ApplyOptions, FramingScale, Front};

/// What the shelf asks the application to do.
#[derive(Clone, Debug)]
pub enum StudioAction {
    /// Apply this preset, so, and dress the product with its rules or not.
    Apply(Box<PresetFile>, ApplyOptions, bool),
    /// Save the scene's studio as a preset.
    Save(crate::studio_presets::SaveAs),
    /// Pick a package or a preset to install.
    Install,
    /// Save a package of this one somewhere.
    Export(Box<PresetFile>),
    /// Remove one of the person's own, asked and answered.
    Delete(Box<PresetFile>),
    /// The folder the person's presets live in, opened.
    OpenFolder,
}

/// Applying one: the preset and the choices the dialog offers.
struct Applying {
    file: PresetFile,
    options: ApplyOptions,
    dress: bool,
}

#[derive(Default)]
pub struct StudioShelf {
    presets: Vec<PresetFile>,
    problems: Vec<String>,
    loaded: bool,
    query: String,
    thumbnails: HashMap<PathBuf, egui::TextureHandle>,
    applying: Option<Applying>,
    saving: Option<crate::studio_presets::SaveAs>,
    deleting: Option<PresetFile>,
    /// The product's bounding radius, for the size to suggest; set by the
    /// application before the shelf is drawn.
    pub product_radius: Option<f32>,
}

impl StudioShelf {
    /// Read the presets again, at the next draw.
    pub fn reload(&mut self) {
        self.loaded = false;
    }

    fn load(&mut self) {
        if self.loaded {
            return;
        }
        let (found, problems) = presets::list();
        self.presets = found;
        self.problems = problems;
        self.thumbnails.clear();
        self.loaded = true;
    }

    /// The preset of each family framed nearest the product's size.
    fn suggestions(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        let Some(radius) = self.product_radius else { return out };
        let families: std::collections::BTreeSet<&str> = self.presets.iter().map(|p| p.preset.family.as_str()).collect();
        for family in families {
            let sizes: Vec<_> = self.presets.iter().filter(|p| p.preset.family == family).map(|p| &p.preset).collect();
            if sizes.len() > 1 {
                if let Some(best) = tequila_scene::preset::nearest_size(sizes, radius) {
                    out.insert(best.id.clone());
                }
            }
        }
        out
    }

    fn thumbnail(&mut self, ctx: &egui::Context, path: &PathBuf) -> Option<egui::TextureHandle> {
        if let Some(handle) = self.thumbnails.get(path) {
            return Some(handle.clone());
        }
        let bytes = std::fs::read(path).ok()?;
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        // Eight bits a channel whatever was saved: a preset's renders, and
        // so a thumbnail made from one, may well be 16-bit.
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().ok()?;
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).ok()?;
        let pixels = &buffer[..info.buffer_size()];
        let rgba: Vec<u8> = match info.color_type {
            png::ColorType::Rgba => pixels.to_vec(),
            png::ColorType::Rgb => pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            png::ColorType::GrayscaleAlpha => pixels.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
            png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::Indexed => return None,
        };
        let image = egui::ColorImage::from_rgba_unmultiplied([info.width as usize, info.height as usize], &rgba);
        let handle = ctx.load_texture(format!("studio-preset-{}", path.display()), image, egui::TextureOptions::LINEAR);
        self.thumbnails.insert(path.clone(), handle.clone());
        Some(handle)
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<StudioAction> {
        self.load();
        let mut action = None;
        ui.label(
            egui::RichText::new("Complete studios - lights, cameras and views, floor, look - applied to the product in one step.")
                .small()
                .color(crate::theme::dim()),
        );
        ui.horizontal(|ui| {
            if ui.button("Save current studio…").on_hover_text("Everything but the product, as a preset of your own.").clicked() {
                self.saving = Some(crate::studio_presets::SaveAs {
                    name: String::new(),
                    family: String::new(),
                    size: String::new(),
                    description: String::new(),
                    materials: false,
                });
            }
            if ui.button("Install…").on_hover_text("A studio preset package (.tqstudio) or a preset's .studio.json").clicked() {
                action = Some(StudioAction::Install);
            }
            if ui.small_button("Folder").on_hover_text("Where your presets are kept").clicked() {
                action = Some(StudioAction::OpenFolder);
            }
        });
        crate::theme::search_field(ui, &mut self.query, "Search studios");
        ui.add_space(4.0);

        let query = self.query.to_lowercase();
        let suggested = self.suggestions();
        let visible: Vec<PresetFile> = self
            .presets
            .iter()
            .filter(|p| {
                query.is_empty()
                    || [&p.preset.name, &p.preset.family, &p.preset.size, &p.preset.description]
                        .iter()
                        .any(|s| s.to_lowercase().contains(&query))
                    || p.preset.tags.iter().any(|t| t.to_lowercase().contains(&query))
            })
            .cloned()
            .collect();
        if visible.is_empty() {
            ui.add_space(12.0);
            ui.label(egui::RichText::new(if self.presets.is_empty() { "No studio presets yet. Install one, or save the studio you have." } else { "None match." }).color(crate::theme::dim()));
        }
        let width = ui.available_width();
        let tile = ((width - 8.0) / 2.0).floor().max(60.0);
        let mut family: Option<String> = None;
        let mut row: Vec<PresetFile> = Vec::new();
        let mut rows: Vec<(Option<String>, Vec<PresetFile>)> = Vec::new();
        for file in visible {
            if family.as_deref() != Some(file.preset.family.as_str()) {
                if !row.is_empty() {
                    rows.push((None, std::mem::take(&mut row)));
                }
                family = Some(file.preset.family.clone());
                rows.push((Some(if file.preset.family.is_empty() { "Studios".into() } else { file.preset.family.clone() }), Vec::new()));
            }
            row.push(file);
            if row.len() == 2 {
                rows.push((None, std::mem::take(&mut row)));
            }
        }
        if !row.is_empty() {
            rows.push((None, row));
        }
        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            for (header, files) in rows {
                if let Some(header) = header {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(header).strong());
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for file in files {
                        let picture = file.thumbnail.clone().and_then(|t| self.thumbnail(ui.ctx(), &t));
                        let response = ui
                            .vertical(|ui| {
                                ui.set_width(tile);
                                let (rect, response) = ui.allocate_exact_size(egui::vec2(tile, tile * 0.75), egui::Sense::click());
                                match picture {
                                    Some(texture) => {
                                        egui::Image::new(&texture).rounding(egui::Rounding::same(7.0)).paint_at(ui, rect);
                                    }
                                    None => {
                                        ui.painter().rect_filled(rect, 7.0, crate::theme::p().control);
                                        crate::icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(tile * 0.3)), crate::icons::Icon::Studios, crate::theme::p().line_strong);
                                    }
                                }
                                if suggested.contains(&file.preset.id) {
                                    let badge = egui::Rect::from_min_size(rect.left_top() + egui::vec2(5.0, 5.0), egui::vec2(66.0, 16.0));
                                    ui.painter().rect_filled(badge, 8.0, crate::theme::p().accent);
                                    ui.painter().text(badge.center(), egui::Align2::CENTER_CENTER, "Suggested", egui::FontId::proportional(10.0), egui::Color32::WHITE);
                                }
                                let title = if file.preset.size.is_empty() { file.preset.name.clone() } else { file.preset.size.clone() };
                                ui.add(egui::Label::new(egui::RichText::new(title).small()).truncate());
                                response
                            })
                            .inner;
                        let response = response.on_hover_text(format!(
                            "{}\n{}\n{} views, {} lights, {} materials",
                            file.preset.name,
                            file.preset.description,
                            file.preset.studios.len(),
                            file.preset.lights.len(),
                            file.preset.materials.len()
                        ));
                        if response.clicked() {
                            self.applying = Some(Applying { file: file.clone(), options: ApplyOptions::default(), dress: false });
                        }
                    }
                });
            }
            for problem in &self.problems {
                ui.label(egui::RichText::new(problem).small().color(crate::theme::p().danger));
            }
        });

        if let Some(asked) = self.apply_dialog(ui.ctx()) {
            action = Some(asked);
        }
        if let Some(asked) = self.save_dialog(ui.ctx()) {
            action = Some(asked);
        }
        if let Some(file) = self.deleting.clone() {
            let mut open = true;
            let mut answer = None;
            egui::Window::new("Delete studio preset").collapsible(false).resizable(false).open(&mut open).show(ui.ctx(), |ui| {
                ui.label(format!("Delete {}? Its folder goes; the materials it installed stay in your library.", file.preset.name));
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Keep").clicked() {
                        answer = Some(false);
                    }
                });
            });
            if answer == Some(true) {
                action = Some(StudioAction::Delete(Box::new(file)));
            }
            if answer.is_some() || !open {
                self.deleting = None;
            }
        }
        action
    }

    fn apply_dialog(&mut self, ctx: &egui::Context) -> Option<StudioAction> {
        let applying = self.applying.as_mut()?;
        let mut open = true;
        let mut action = None;
        let mut close = false;
        let preset = applying.file.preset.clone();
        egui::Window::new(preset.name.clone()).collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
            ui.set_max_width(300.0);
            if !preset.description.is_empty() {
                ui.label(egui::RichText::new(&preset.description).small().color(crate::theme::dim()));
            }
            let views: Vec<&str> = preset.studios.iter().map(|s| s.name.as_str()).collect();
            ui.label(egui::RichText::new(format!("Views: {}", if views.is_empty() { "none".into() } else { views.join(", ") })).small());
            ui.add_space(6.0);
            ui.label("The product's front faces");
            let options: Vec<(Front, &str)> = vec![(Front::PlusZ, "+Z"), (Front::MinusZ, "−Z"), (Front::PlusX, "+X"), (Front::MinusX, "−X")];
            crate::theme::segmented(ui, &mut applying.options.front, &options);
            ui.label(
                egui::RichText::new("CAD files arrive any way round: the side that should face the Main camera.")
                    .small()
                    .color(crate::theme::dim()),
            );
            ui.add_space(4.0);
            ui.label("Scaled to the product");
            let mut scale = applying.options.scale.unwrap_or(preset.framing.scale);
            let scales: Vec<(FramingScale, &str)> =
                vec![(FramingScale::Cameras, "Cameras"), (FramingScale::CamerasAndLights, "Cameras and lights"), (FramingScale::None, "Nothing")];
            if crate::theme::segmented(ui, &mut scale, &scales) {
                applying.options.scale = Some(scale);
            }
            ui.add(crate::field::Checkbox::new(&mut applying.options.keep_lights, "Keep my lights"));
            ui.add(crate::field::Checkbox::new(&mut applying.options.keep_cameras, "Keep my cameras too"));
            ui.add(crate::field::Checkbox::new(&mut applying.options.keep_placement, "Leave the product where it is"));
            if !preset.material_rules.is_empty() {
                ui.add(crate::field::Checkbox::new(
                    &mut applying.dress,
                    format!("Dress it with the studio's materials ({} rules)", preset.material_rules.len()),
                ))
                .on_hover_text("By part name: the preset's rules, as a Material Template. Off, the materials are added to assign yourself.");
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new("Apply").strong()).clicked() {
                    action = Some(StudioAction::Apply(Box::new(applying.file.clone()), applying.options, applying.dress));
                    close = true;
                }
                if ui.button("Export…").clicked() {
                    action = Some(StudioAction::Export(Box::new(applying.file.clone())));
                }
                if applying.file.source == PresetSource::User && ui.button("Delete…").clicked() {
                    self.deleting = Some(applying.file.clone());
                    close = true;
                }
            });
        });
        if close || !open {
            self.applying = None;
        }
        action
    }

    fn save_dialog(&mut self, ctx: &egui::Context) -> Option<StudioAction> {
        let saving = self.saving.as_mut()?;
        let mut open = true;
        let mut action = None;
        let mut close = false;
        egui::Window::new("Save studio as preset").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
            ui.set_max_width(320.0);
            ui.label(
                egui::RichText::new("Everything but the product: lights, cameras and Studios, environment and floor, look, render and lighting settings - framed for the product you have now.")
                    .small()
                    .color(crate::theme::dim()),
            );
            egui::Grid::new("save-studio-preset").num_columns(2).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut saving.name);
                ui.end_row();
                ui.label("Family");
                ui.add(egui::TextEdit::singleline(&mut saving.family).hint_text("Product Studio"));
                ui.end_row();
                ui.label("Size");
                ui.add(egui::TextEdit::singleline(&mut saving.size).hint_text("Medium"));
                ui.end_row();
                ui.label("Description");
                ui.text_edit_singleline(&mut saving.description);
                ui.end_row();
            });
            ui.add(crate::field::Checkbox::new(&mut saving.materials, "Keep this project's materials with it"));
            ui.horizontal(|ui| {
                let ready = !saving.name.trim().is_empty();
                if ui.add_enabled(ready, egui::Button::new(egui::RichText::new("Save").strong())).clicked() {
                    action = Some(StudioAction::Save(saving.clone()));
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close || !open {
            self.saving = None;
        }
        action
    }
}
