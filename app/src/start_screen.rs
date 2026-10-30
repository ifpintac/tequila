//! The start screen: what greets the person at launch, and is under Help →
//! Start screen. A new project, one to open or a model to import; the
//! projects opened lately, each with the picture it was saved with; two
//! finished scenes to start from; which version this is, and where to talk
//! about it.

use crate::{icons, theme};
use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// The community and the source, on the start screen and in Help.
pub const DISCORD: &str = "https://discord.gg/Na5T2qgQBX";
pub const GITHUB: &str = "https://github.com/ifpintac/tequila";
pub const GUIDES: &str = "https://github.com/ifpintac/tequila/tree/main/docs";
pub const RELEASES: &str = "https://github.com/ifpintac/tequila/releases";

/// A finished scene that ships with Tequila, to look round or start from.
pub struct Sample {
    pub name: &'static str,
    pub about: &'static str,
    /// Where it is shipped, beside the application or in the checkout.
    pub file: &'static str,
}

pub const SAMPLES: [Sample; 2] = [
    Sample {
        name: "Headphones",
        about: "Soft-touch, protein leather and brushed steel, rim-lit on a grey seamless",
        file: "assets/samples/Headphones.tequila",
    },
    Sample {
        name: "Perfume bottle",
        about: "Heavy flint glass round amber parfum, gold hardware, backlit in the dark",
        file: "assets/samples/Perfume bottle.tequila",
    },
];

/// What the person chose.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    New,
    Open,
    Import,
    Recent(PathBuf),
    /// Take a recent project off the list.
    Forget(PathBuf),
    /// Open a sample as a new, unsaved project.
    Sample(PathBuf),
    /// See the newer version, and install it.
    Update,
}

enum Picture {
    Reading,
    None,
    Shown(egui::TextureHandle),
}

#[derive(Default)]
pub struct StartScreen {
    pub open: bool,
    /// The projects' pictures, read off the interface's thread.
    pictures: HashMap<PathBuf, Picture>,
    arriving: Option<mpsc::Receiver<(PathBuf, Option<egui::ColorImage>)>>,
}

impl StartScreen {
    /// Draw it while it is open: the choice made, if one was. Choosing
    /// anything but forgetting a project puts it away.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        recent: &[PathBuf],
        show_at_startup: &mut bool,
        newer: Option<crate::update::Version>,
    ) -> Option<Choice> {
        if !self.open {
            return None;
        }
        let samples: Vec<(&Sample, Option<PathBuf>)> =
            SAMPLES.iter().map(|s| (s, crate::shipped_file(s.file, s.file))).collect();
        self.read_pictures(recent.iter().cloned().chain(samples.iter().filter_map(|(_, p)| p.clone())).collect());
        self.receive(ctx);

        let p = theme::p();
        let screen = ctx.screen_rect();
        let mut choice = None;
        let mut dismiss = ctx.input(|i| i.key_pressed(egui::Key::Escape));

        // The rest of the window, dimmed, and closed by a click.
        egui::Area::new(egui::Id::new("start-screen-backdrop"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(screen, egui::Sense::click());
                ui.painter().rect_filled(screen, 0.0, egui::Color32::from_black_alpha(if p.dark { 150 } else { 90 }));
                if response.clicked() {
                    dismiss = true;
                }
            });

        let size = egui::vec2(940.0_f32.min(screen.width() - 32.0), 600.0_f32.min(screen.height() - 32.0));
        let card = egui::Rect::from_center_size(screen.center(), size);
        egui::Area::new(egui::Id::new("start-screen"))
            .order(egui::Order::Foreground)
            .fixed_pos(card.min)
            .show(ctx, |ui| {
                ui.painter().add(
                    egui::epaint::Shadow { offset: egui::vec2(0.0, 18.0), blur: 48.0, spread: 0.0, color: egui::Color32::from_black_alpha(if p.dark { 140 } else { 60 }) }
                        .as_shape(card, 18.0),
                );
                ui.painter().rect(card, 18.0, p.raised, egui::Stroke::new(1.0_f32, p.line));
                // Clicks on the card stay on it.
                let _ = ui.allocate_rect(card, egui::Sense::click());
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(card.shrink2(egui::vec2(32.0, 26.0))), |ui| {
                    if header(ui) {
                        dismiss = true;
                    }
                    ui.add_space(18.0);
                    let footer_height = 44.0;
                    let body = egui::Rect::from_min_max(ui.cursor().min, egui::pos2(ui.max_rect().right(), ui.max_rect().bottom() - footer_height));
                    let gap = 32.0;
                    let left_width = (body.width() * 0.4).clamp(280.0, 340.0);
                    let left = egui::Rect::from_min_size(body.min, egui::vec2(left_width, body.height()));
                    let right = egui::Rect::from_min_max(egui::pos2(left.right() + gap, body.top()), body.max);
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(left), |ui| {
                        choice = choice.take().or(self.start_column(ui, &samples));
                    });
                    ui.painter().vline(left.right() + gap * 0.5, body.y_range(), egui::Stroke::new(1.0_f32, p.line));
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(right), |ui| {
                        choice = choice.take().or(self.recent_column(ui, recent));
                    });
                    let footer = egui::Rect::from_min_max(egui::pos2(body.left(), body.bottom()), ui.max_rect().max);
                    ui.painter().hline(footer.x_range(), footer.top(), egui::Stroke::new(1.0_f32, p.line));
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(footer.shrink2(egui::vec2(0.0, 6.0))), |ui| {
                        if footer_row(ui, show_at_startup, newer) {
                            choice = Some(Choice::Update);
                        }
                    });
                });
            });

        if choice.as_ref().is_some_and(|c| !matches!(c, Choice::Forget(_))) || dismiss {
            self.open = false;
        }
        choice
    }

    fn start_column(&mut self, ui: &mut egui::Ui, samples: &[(&Sample, Option<PathBuf>)]) -> Option<Choice> {
        let mut choice = None;
        heading(ui, "Start");
        if action(ui, icons::Icon::FilePlus, "New project", &theme::keys("Ctrl+N")).clicked() {
            choice = Some(Choice::New);
        }
        if action(ui, icons::Icon::FolderOpen, "Open project...", &theme::keys("Ctrl+O")).clicked() {
            choice = Some(Choice::Open);
        }
        if action(ui, icons::Icon::Import, "Import model...", "OBJ, STEP, glTF").clicked() {
            choice = Some(Choice::Import);
        }
        if samples.iter().all(|(_, path)| path.is_none()) {
            return choice;
        }
        ui.add_space(18.0);
        heading(ui, "Samples");
        let gap = 12.0;
        let width = ((ui.available_width() - gap) / 2.0).max(80.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (sample, path) in samples {
                if let Some(path) = path {
                    let picture = match self.pictures.get(path) {
                        Some(Picture::Shown(texture)) => Some(texture.clone()),
                        _ => None,
                    };
                    if sample_tile(ui, width, sample, picture.as_ref()).clicked() {
                        choice = Some(Choice::Sample(path.clone()));
                    }
                }
            }
        });
        choice
    }

    fn recent_column(&mut self, ui: &mut egui::Ui, recent: &[PathBuf]) -> Option<Choice> {
        let p = theme::p();
        let mut choice = None;
        heading(ui, "Recent");
        if recent.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                let (mark, _) = ui.allocate_exact_size(egui::Vec2::splat(30.0), egui::Sense::hover());
                icons::paint(ui.painter(), mark, icons::Icon::Clock, p.text_faint);
                ui.add_space(6.0);
                ui.label(egui::RichText::new("The projects you open and save show here").size(13.0).color(p.text_dim));
            });
            return None;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for path in recent {
                let picture = match self.pictures.get(path) {
                    Some(Picture::Shown(texture)) => Some(texture.clone()),
                    _ => None,
                };
                match recent_row(ui, path, picture.as_ref()) {
                    Row::Open => choice = Some(Choice::Recent(path.clone())),
                    Row::Forget => choice = Some(Choice::Forget(path.clone())),
                    Row::Nothing => {}
                }
            }
        });
        choice
    }

    /// Start reading the pictures of `paths` it has not got.
    fn read_pictures(&mut self, paths: Vec<PathBuf>) {
        let wanted: Vec<PathBuf> = paths.into_iter().filter(|p| !self.pictures.contains_key(p)).collect();
        if wanted.is_empty() || self.arriving.is_some() {
            return;
        }
        for path in &wanted {
            self.pictures.insert(path.clone(), Picture::Reading);
        }
        let (send, receive) = mpsc::channel();
        self.arriving = Some(receive);
        std::thread::spawn(move || {
            for path in wanted {
                let picture = tequila_format::read_preview(&path).and_then(|bytes| decode(&bytes));
                if send.send((path, picture)).is_err() {
                    return;
                }
            }
        });
    }

    fn receive(&mut self, ctx: &egui::Context) {
        let Some(arriving) = &self.arriving else { return };
        loop {
            match arriving.try_recv() {
                Ok((path, picture)) => {
                    let shown = match picture {
                        Some(image) => Picture::Shown(ctx.load_texture(format!("start-{}", path.display()), image, egui::TextureOptions::LINEAR)),
                        None => Picture::None,
                    };
                    self.pictures.insert(path, shown);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.arriving = None;
                    return;
                }
            }
        }
    }

    /// Forget a project's picture, so the next look reads it again: it has
    /// just been saved with a new one.
    pub fn refresh(&mut self, path: &Path) {
        if !matches!(self.pictures.get(path), Some(Picture::Reading)) {
            self.pictures.remove(path);
        }
    }
}

/// The brand, what Tequila is, and the version: true when it is closed.
fn header(ui: &mut egui::Ui) -> bool {
    let p = theme::p();
    let mut close = false;
    ui.horizontal(|ui| {
        let (mark, _) = ui.allocate_exact_size(egui::Vec2::splat(40.0), egui::Sense::hover());
        icons::paint(ui.painter(), mark, icons::Icon::Aperture, p.accent);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(egui::RichText::new("Welcome to Tequila").font(theme::strong(22.0)).color(p.text));
            ui.label(egui::RichText::new(format!("Product rendering · {}", crate::update::label())).size(12.5).color(p.text_dim));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if icons::button(ui, icons::Icon::Close, false, 28.0, egui::Sense::click()).on_hover_text("Close").clicked() {
                close = true;
            }
        });
    });
    ui.add_space(10.0);
    ui.label(
        egui::RichText::new(
            "Photoreal pictures of your products. Bring in a CAD file or a model, dress it from the material \
             library, light it in a studio, and render it in Tequila's own renderer or in Cycles on your \
             graphics card.",
        )
        .size(13.5)
        .color(p.text_dim),
    );
    close
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).font(theme::strong(11.0)).color(theme::p().text_faint));
    ui.add_space(6.0);
}

/// A row to click: an icon, what it does, and a hint at the right.
fn action(ui: &mut egui::Ui, icon: icons::Icon, label: &str, hint: &str) -> egui::Response {
    let p = theme::p();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 9.0, p.hover);
    }
    let glyph = egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), egui::Vec2::splat(18.0));
    icons::paint(ui.painter(), glyph, icon, if response.hovered() { p.accent } else { p.icon });
    ui.painter().text(egui::pos2(rect.left() + 40.0, rect.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(14.0), p.text);
    ui.painter().text(egui::pos2(rect.right() - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, hint, egui::FontId::proportional(11.5), p.text_faint);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn sample_tile(ui: &mut egui::Ui, width: f32, sample: &Sample, picture: Option<&egui::TextureHandle>) -> egui::Response {
    let p = theme::p();
    let picture_height = (width * 0.75).round();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, picture_height + 30.0), egui::Sense::click());
    let frame = egui::Rect::from_min_size(rect.min, egui::vec2(width, picture_height));
    draw_picture(ui, frame, picture, icons::Icon::Gem, 10.0);
    if response.hovered() {
        ui.painter().rect_stroke(frame, 10.0, egui::Stroke::new(2.0_f32, p.accent));
    }
    ui.painter().text(egui::pos2(rect.left() + 2.0, frame.bottom() + 15.0), egui::Align2::LEFT_CENTER, sample.name, theme::strong(13.0), p.text);
    response.on_hover_text(sample.about).on_hover_cursor(egui::CursorIcon::PointingHand)
}

enum Row {
    Open,
    Forget,
    Nothing,
}

fn recent_row(ui: &mut egui::Ui, path: &Path, picture: Option<&egui::TextureHandle>) -> Row {
    let p = theme::p();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::click());
    let there = path.is_file();
    if response.hovered() {
        ui.painter().rect_filled(rect, 10.0, p.hover);
    }
    let frame = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 8.0), egui::vec2(77.0, 48.0));
    draw_picture(ui, frame, picture, icons::Icon::Aperture, 6.0);
    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let text_left = frame.right() + 14.0;
    ui.painter().text(egui::pos2(text_left, rect.top() + 22.0), egui::Align2::LEFT_CENTER, name, theme::strong(13.5), if there { p.text } else { p.text_faint });
    let detail = if there {
        let folder = path.parent().map(short_folder).unwrap_or_default();
        match modified_ago(path) {
            Some(age) => format!("{folder}  ·  {age}"),
            None => folder,
        }
    } else {
        "No longer there".to_string()
    };
    ui.painter().text(egui::pos2(text_left, rect.top() + 42.0), egui::Align2::LEFT_CENTER, detail, egui::FontId::proportional(12.0), p.text_dim);

    let mut row = Row::Nothing;
    if response.hovered() {
        let forget = egui::Rect::from_center_size(egui::pos2(rect.right() - 22.0, rect.center().y), egui::Vec2::splat(24.0));
        let hit = ui.interact(forget, ui.id().with(("forget", path)), egui::Sense::click());
        icons::paint(ui.painter(), forget.shrink(5.0), icons::Icon::Close, if hit.hovered() { p.text } else { p.text_faint });
        if hit.on_hover_text("Take it off the list").clicked() {
            row = Row::Forget;
        }
    }
    if matches!(row, Row::Nothing) && response.on_hover_text(path.display().to_string()).clicked() {
        row = Row::Open;
    }
    row
}

/// A picture in `frame`, or a quiet stand-in while there is none.
fn draw_picture(ui: &egui::Ui, frame: egui::Rect, picture: Option<&egui::TextureHandle>, stand_in: icons::Icon, rounding: f32) {
    let p = theme::p();
    match picture {
        Some(texture) => {
            // Cover the frame, cropping what overhangs, as a photo is shown.
            let size = texture.size_vec2();
            let scale = (frame.width() / size.x).max(frame.height() / size.y);
            let shown = size * scale;
            let uv_size = egui::vec2(frame.width() / shown.x, frame.height() / shown.y);
            let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), uv_size);
            egui::Image::new((texture.id(), frame.size())).uv(uv).rounding(rounding).paint_at(ui, frame);
        }
        None => {
            ui.painter().rect_filled(frame, rounding, p.field);
            let glyph = egui::Rect::from_center_size(frame.center(), egui::Vec2::splat((frame.height() * 0.36).min(28.0)));
            icons::paint(ui.painter(), glyph, stand_in, p.text_faint);
        }
    }
}

/// The links, the startup switch, and a newer version when there is one:
/// true when that was clicked.
fn footer_row(ui: &mut egui::Ui, show_at_startup: &mut bool, newer: Option<crate::update::Version>) -> bool {
    let mut update = false;
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (icon, label, url, about) in [
            (icons::Icon::Discord, "Discord", DISCORD, "Questions, renders and news, with the people who make Tequila"),
            (icons::Icon::Github, "GitHub", GITHUB, "The source, and where to report a bug"),
            (icons::Icon::Book, "Guides", GUIDES, "How Tequila works, in depth"),
            (icons::Icon::ExternalLink, "What's new", RELEASES, "Every release and what it brought"),
        ] {
            if icons::labelled(ui, icon, label, false).on_hover_text(about).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(crate::field::Checkbox::new(show_at_startup, "Show at startup"));
            if let Some(version) = newer {
                ui.add_space(10.0);
                let label = format!("Update to {version}");
                if icons::labelled(ui, icons::Icon::Update, &label, true)
                    .on_hover_text("A newer Tequila is out: what it brings, and installing it")
                    .clicked()
                {
                    update = true;
                }
            }
        });
    });
    update
}

/// The last two folders of `folder`, which say where it is more briefly
/// than the whole path.
fn short_folder(folder: &Path) -> String {
    let parts: Vec<String> = folder.components().rev().take(2).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    match parts.as_slice() {
        [last, before] if !before.ends_with('\\') && !before.ends_with('/') => format!("{before}/{last}"),
        [last, ..] => last.clone(),
        [] => String::new(),
    }
}

fn modified_ago(path: &Path) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(ago(std::time::SystemTime::now().duration_since(modified).unwrap_or_default().as_secs()))
}

/// How long ago, in words.
fn ago(seconds: u64) -> String {
    let plural = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match seconds {
        0..=59 => "Just now".into(),
        60..=3599 => plural(seconds / 60, "minute"),
        3600..=86_399 => plural(seconds / 3600, "hour"),
        86_400..=172_799 => "Yesterday".into(),
        172_800..=1_209_599 => plural(seconds / 86_400, "day"),
        1_209_600..=5_183_999 => plural(seconds / 604_800, "week"),
        5_184_000..=31_535_999 => plural(seconds / 2_592_000, "month"),
        _ => plural(seconds / 31_536_000, "year"),
    }
}

/// A picture saved with a project, for the screen.
fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    let data = &buffer[..info.buffer_size()];
    let size = [info.width as usize, info.height as usize];
    match info.color_type {
        png::ColorType::Rgb => Some(egui::ColorImage::from_rgb(size, data)),
        png::ColorType::Rgba => Some(egui::ColorImage::from_rgba_unmultiplied(size, data)),
        _ => None,
    }
}

/// The picture a project is saved with: the viewport as it is, at most 512
/// by 320, as a PNG. `None` while the viewport has shown nothing.
pub fn picture_of(image: &crate::orchestrator::DisplayImage) -> Option<Vec<u8>> {
    let (width, height) = (image.width, image.height);
    if width == 0 || height == 0 || image.pixels.len() < width * height * 4 {
        return None;
    }
    let step = ((width as f32 / 512.0).max(height as f32 / 320.0)).max(1.0);
    let (to_width, to_height) = (((width as f32 / step).round() as usize).max(1), ((height as f32 / step).round() as usize).max(1));
    let mut rgb = Vec::with_capacity(to_width * to_height * 3);
    for y in 0..to_height {
        let (y0, y1) = ((y as f32 * step) as usize, (((y + 1) as f32 * step) as usize).clamp(1, height));
        for x in 0..to_width {
            let (x0, x1) = ((x as f32 * step) as usize, (((x + 1) as f32 * step) as usize).clamp(1, width));
            let mut sum = [0u32; 3];
            let mut count = 0u32;
            for sy in y0..y1.max(y0 + 1) {
                for sx in x0..x1.max(x0 + 1) {
                    let at = (sy.min(height - 1) * width + sx.min(width - 1)) * 4;
                    for c in 0..3 {
                        sum[c] += u32::from(image.pixels[at + c]);
                    }
                    count += 1;
                }
            }
            rgb.extend(sum.map(|s| (s / count.max(1)) as u8));
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, to_width as u32, to_height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgb).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project's picture survives the trip: made from the viewport, saved,
    /// read back alone and decoded, it is the viewport's picture, smaller.
    #[test]
    fn a_projects_picture_is_the_viewport_smaller() {
        let (width, height) = (1024usize, 640usize);
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for _ in 0..width {
                pixels.extend_from_slice(&[if y < height / 2 { 200 } else { 20 }, 90, 40, 255]);
            }
        }
        let image = crate::orchestrator::DisplayImage { width, height, pixels, ..Default::default() };
        let png = picture_of(&image).expect("a picture");
        let decoded = decode(&png).expect("it decodes");
        assert_eq!(decoded.size, [512, 320]);
        assert_eq!(decoded.pixels[0], egui::Color32::from_rgb(200, 90, 40));
        assert_eq!(decoded.pixels[512 * 319], egui::Color32::from_rgb(20, 90, 40));
        assert!(picture_of(&crate::orchestrator::DisplayImage::default()).is_none());
    }

    /// Time in words, at each boundary.
    #[test]
    fn how_long_ago_in_words() {
        assert_eq!(ago(5), "Just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(7200), "2 hours ago");
        assert_eq!(ago(90_000), "Yesterday");
        assert_eq!(ago(3 * 86_400), "3 days ago");
        assert_eq!(ago(21 * 86_400), "3 weeks ago");
        assert_eq!(ago(400 * 86_400), "1 year ago");
    }

    /// A folder is named by its last two parts.
    #[test]
    fn a_folder_is_named_briefly() {
        assert_eq!(short_folder(Path::new("/home/someone/Projects/Shoes")), "Projects/Shoes");
        assert_eq!(short_folder(Path::new("/Shoes")), "Shoes");
    }
}
