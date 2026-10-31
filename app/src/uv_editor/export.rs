//! Export layout: the layout written as a picture to draw artwork over in a
//! drawing program - an SVG of its lines, or a PNG drawn from it - by the
//! layout engine's `layout_svg` and `layout_png`, as the scripting channel's
//! `uv_layout` writes it. The dialog sets its size, what fills the islands,
//! the lines' colour and whether the square under them is white or clear,
//! with a small picture of what it will write.

use tequila_scene::uv::{layout_png, layout_svg, Fill, LayoutPicture, Measure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Svg,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Svg => "svg",
        }
    }

    pub fn of_path(path: &std::path::Path) -> Option<Format> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "png" => Some(Format::Png),
            "svg" => Some(Format::Svg),
            _ => None,
        }
    }
}

/// The dialog's settings, kept while the editor is open.
#[derive(Clone)]
pub struct Dialog {
    pub open: bool,
    pub format: Format,
    pub size: u32,
    pub fill: Fill,
    /// The lines in a colour of their own, and which.
    pub coloured: bool,
    pub line: egui::Color32,
    pub transparent: bool,
    pub edges: bool,
    pub seams: bool,
    pub note: bool,
    /// The small picture, and what it was drawn for.
    preview: Option<(PreviewKey, egui::TextureHandle)>,
}

type PreviewKey = (tequila_scene::MeshKey, usize, String);

impl Default for Dialog {
    fn default() -> Self {
        Dialog {
            open: false,
            format: Format::Png,
            size: 2048,
            fill: Fill::Islands,
            coloured: false,
            line: egui::Color32::from_rgb(29, 32, 36),
            transparent: false,
            edges: true,
            seams: true,
            note: true,
            preview: None,
        }
    }
}

/// What the dialog asked for.
pub enum Asked {
    Nothing,
    /// Write it, with these settings, where a file dialog says.
    Save,
}

impl Dialog {
    pub fn picture(&self) -> LayoutPicture {
        LayoutPicture {
            size: self.size,
            fill: self.fill,
            edges: self.edges,
            seams: self.seams,
            note: self.note,
            line: self.coloured.then(|| [self.line.r(), self.line.g(), self.line.b()]),
            transparent: self.transparent,
        }
    }

    /// Set from a script's request, as the dialog's fields would be.
    pub fn take(&mut self, request: &tequila_scripting::protocol::UvExport) -> Result<(), String> {
        if let Some(size) = request.size {
            self.size = size.clamp(16, tequila_scene::uv::export::MOST_PIXELS);
        }
        if let Some(fill) = &request.fill {
            self.fill = match fill.as_str() {
                "islands" => Fill::Islands,
                "none" => Fill::None,
                "area" => Fill::Distortion(Measure::Area),
                "angle" => Fill::Distortion(Measure::Angle),
                other => return Err(format!("`{other}` is not a fill: islands, none, area or angle")),
            };
        }
        if let Some(line) = &request.line {
            let [r, g, b] = tequila_scripting::uv_layout::colour(line).ok_or_else(|| format!("`{line}` is not a colour: give one as \"#rrggbb\""))?;
            self.coloured = true;
            self.line = egui::Color32::from_rgb(r, g, b);
        }
        if let Some(clear) = request.transparent {
            self.transparent = clear;
        }
        if let Some(format) = Format::of_path(std::path::Path::new(&request.path)) {
            self.format = format;
        }
        Ok(())
    }

    /// The dialog, over the editor. `mesh` is the part's, and `set` the UV
    /// set shown.
    pub fn show(&mut self, ctx: &egui::Context, mesh: Option<(tequila_scene::MeshKey, &tequila_scene::Mesh)>, set: usize) -> Asked {
        if !self.open {
            return Asked::Nothing;
        }
        let mut asked = Asked::Nothing;
        let mut open = true;
        let p = crate::theme::p();
        egui::Window::new("Export layout")
            .id(egui::Id::new("uv-export-layout"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .frame(egui::Frame::window(&ctx.style()).fill(p.raised).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(240.0);
                        crate::theme::section(ui, "Format");
                        crate::theme::segmented(ui, &mut self.format, &[(Format::Png, "PNG"), (Format::Svg, "SVG")]);
                        crate::theme::section(ui, "Size");
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut self.size).range(16..=tequila_scene::uv::export::MOST_PIXELS).speed(8.0).suffix(" px"))
                                .on_hover_text("The layout's square, this many pixels a side");
                            for size in [1024u32, 2048, 4096] {
                                if ui.selectable_label(self.size == size, format!("{}K", size / 1024)).clicked() {
                                    self.size = size;
                                }
                            }
                        });
                        crate::theme::section(ui, "Fill");
                        crate::theme::segmented(ui, &mut self.fill, &[(Fill::Islands, "Islands"), (Fill::None, "None"), (Fill::Distortion(Measure::Area), "Area"), (Fill::Distortion(Measure::Angle), "Angle")]);
                        ui.small("Each island a colour of its own, nothing, or how much the layout stretches the part.");
                    });
                    ui.add_space(8.0);
                    self.preview(ui, mesh, set);
                });
                crate::theme::section(ui, "Lines");
                ui.horizontal(|ui| {
                    ui.add(crate::field::Checkbox::new(&mut self.edges, "Every edge")).on_hover_text("The edges inside the islands too, faintly");
                    ui.add(crate::field::Checkbox::new(&mut self.seams, "Seams in colour")).on_hover_text("Where the layout is cut, in orange");
                });
                ui.horizontal(|ui| {
                    ui.add(crate::field::Checkbox::new(&mut self.coloured, "Line colour")).on_hover_text("Draw the lines in a colour of your own rather than greys");
                    ui.add_enabled_ui(self.coloured, |ui| {
                        egui::color_picker::color_edit_button_srgba(ui, &mut self.line, egui::color_picker::Alpha::Opaque);
                    });
                });
                crate::theme::section(ui, "Background");
                ui.horizontal(|ui| {
                    ui.add(crate::field::Checkbox::new(&mut self.transparent, "Transparent")).on_hover_text("No white square under the layout: clear where nothing is drawn, to lay over artwork");
                    ui.add(crate::field::Checkbox::new(&mut self.note, "Size note")).on_hover_text("A note in the corner saying how large the square is on the part, where the layout is one size all over");
                });
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save = egui::Button::new(egui::RichText::new("Save…").color(p.on_accent)).fill(p.accent);
                    if ui.add_enabled(mesh.is_some(), save).on_hover_text("Choose where to write it").clicked() {
                        asked = Asked::Save;
                    }
                    if ui.button("Cancel").clicked() {
                        self.open = false;
                    }
                });
            });
        if !open {
            self.open = false;
        }
        asked
    }

    /// The picture as it will be written, small.
    fn preview(&mut self, ui: &mut egui::Ui, mesh: Option<(tequila_scene::MeshKey, &tequila_scene::Mesh)>, set: usize) {
        let p = crate::theme::p();
        let side = 160.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
        // A chequer under it, so a clear background shows as clear.
        let cell = side / 10.0;
        for y in 0..10 {
            for x in 0..10 {
                let shade = if (x + y) % 2 == 0 { p.field } else { p.control };
                let r = egui::Rect::from_min_size(rect.min + egui::vec2(x as f32 * cell, y as f32 * cell), egui::Vec2::splat(cell));
                ui.painter().rect_filled(r, 0.0, shade);
            }
        }
        let Some((key, mesh)) = mesh else { return };
        let picture = LayoutPicture { size: 256, note: false, ..self.picture() };
        let wanted: PreviewKey = (key, set, format!("{picture:?}"));
        if self.preview.as_ref().map(|(k, _)| k) != Some(&wanted) {
            self.preview = layout_png(mesh, set, &picture).ok().and_then(|png| decode(&png)).map(|image| (wanted, ui.ctx().load_texture("uv-export-preview", image, egui::TextureOptions::LINEAR)));
        }
        if let Some((_, texture)) = &self.preview {
            ui.painter().image(texture.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0_f32, p.line_strong));
    }
}

/// A PNG's pixels, for the preview.
fn decode(png: &[u8]) -> Option<egui::ColorImage> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([info.width as usize, info.height as usize], &buffer[..info.buffer_size()]))
}

/// The layout written to `path`, as the path's extension says: how many
/// bytes.
pub fn write(mesh: &tequila_scene::Mesh, set: usize, picture: &LayoutPicture, path: &std::path::Path) -> Result<usize, String> {
    let bytes = match Format::of_path(path) {
        Some(Format::Svg) => layout_svg(mesh, set, picture).map_err(|e| e.to_string())?.into_bytes(),
        Some(Format::Png) => layout_png(mesh, set, picture)?,
        None => return Err(format!("{} is not a .png or an .svg", path.display())),
    };
    std::fs::write(path, &bytes).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(bytes.len())
}

/// Where to write it, chosen.
pub fn pick_path(name: &str, format: Format) -> Option<std::path::PathBuf> {
    let file = format!("{name} UV layout.{}", format.extension());
    let dialog = rfd::FileDialog::new().set_file_name(file);
    match format {
        Format::Png => dialog.add_filter("PNG picture", &["png"]),
        Format::Svg => dialog.add_filter("SVG drawing", &["svg"]),
    }
    .save_file()
}
