//! The start screen: what greets the person at launch, and is under Help →
//! Start screen. A banner of one of the samples with the welcome on it; a
//! new project, one to open or a model to import; the projects opened
//! lately, each with the picture it was saved with; the two finished
//! scenes to start from; and where to talk about Tequila.

use crate::{icons, theme};
use eframe::egui;
use egui::{Color32, Rect, Rounding, Stroke, Vec2};
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
    /// Its wide shot, for the banner (`scripts/samples/make_banners.py`).
    banner: &'static [u8],
}

pub const SAMPLES: [Sample; 2] = [
    Sample {
        name: "Headphones",
        about: "Soft-touch, protein leather and brushed steel, rim-lit in the dark",
        file: "assets/samples/Headphones.tequila",
        banner: include_bytes!("../assets/start/headphones.png"),
    },
    Sample {
        name: "Perfume bottle",
        about: "Flint glass round amber parfum and gold, backlit by a warm glow",
        file: "assets/samples/Perfume bottle.tequila",
        banner: include_bytes!("../assets/start/perfume-bottle.png"),
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
    /// Today's banner, and which sample it shows.
    banner: Option<(usize, Option<egui::TextureHandle>)>,
}

/// The card's measures.
const RADIUS: f32 = 18.0;
const PAD: f32 = 30.0;
const GAP: f32 = 14.0;

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
        let banner = self.banner(ctx);

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
                ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(if p.dark { 160 } else { 110 }));
                if response.clicked() {
                    dismiss = true;
                }
            });

        let size = egui::vec2((screen.width() - 48.0).clamp(640.0, 1080.0), (screen.height() - 48.0).clamp(480.0, 720.0));
        let card = Rect::from_center_size(screen.center(), size);
        egui::Area::new(egui::Id::new("start-screen"))
            .order(egui::Order::Foreground)
            .fixed_pos(card.min)
            .show(ctx, |ui| {
                let painter = ui.painter().clone();
                painter.add(
                    egui::epaint::Shadow { offset: egui::vec2(0.0, 24.0), blur: 64.0, spread: 0.0, color: Color32::from_black_alpha(if p.dark { 150 } else { 70 }) }
                        .as_shape(card, RADIUS),
                );
                painter.rect(card, RADIUS, p.raised, Stroke::new(1.0_f32, p.line));
                // Clicks on the card stay on it.
                let _ = ui.allocate_rect(card, egui::Sense::click());

                // As deep as the banner is for its width, so little of the
                // picture is cut away, and never most of the card.
                let hero_height = (card.width() / 3.6).clamp(170.0, 300.0).min(card.height() * 0.4);
                let hero = Rect::from_min_size(card.min, egui::vec2(card.width(), hero_height));
                if hero_section(ui, hero, banner.as_ref()) {
                    dismiss = true;
                }

                let footer_height = 54.0;
                let inner = Rect::from_min_max(
                    egui::pos2(card.left() + PAD, hero.bottom() + 22.0),
                    egui::pos2(card.right() - PAD, card.bottom() - footer_height),
                );
                let actions = Rect::from_min_size(inner.min, egui::vec2(inner.width(), 74.0));
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(actions), |ui| {
                    choice = choice.take().or(action_cards(ui));
                });

                let body = Rect::from_min_max(egui::pos2(inner.left(), actions.bottom() + 24.0), inner.max);
                let samples_width = (body.width() * 0.34).clamp(250.0, 330.0);
                let recent_area = Rect::from_min_max(body.min, egui::pos2(body.right() - samples_width - PAD, body.bottom()));
                let samples_area = Rect::from_min_max(egui::pos2(body.right() - samples_width, body.top()), body.max);
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(recent_area), |ui| {
                    choice = choice.take().or(self.recent_grid(ui, recent));
                });
                painter.vline(recent_area.right() + PAD * 0.5, body.y_range(), Stroke::new(1.0_f32, p.line));
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(samples_area), |ui| {
                    choice = choice.take().or(self.sample_list(ui, &samples));
                });

                let footer = Rect::from_min_max(egui::pos2(card.left(), card.bottom() - footer_height), card.max);
                painter.hline(footer.x_range(), footer.top(), Stroke::new(1.0_f32, p.line));
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(footer.shrink2(egui::vec2(PAD - 8.0, 10.0))), |ui| {
                    if footer_row(ui, show_at_startup, newer) {
                        choice = Some(Choice::Update);
                    }
                });
            });

        if choice.as_ref().is_some_and(|c| !matches!(c, Choice::Forget(_))) || dismiss {
            self.open = false;
        }
        choice
    }

    /// Today's banner: one sample one day, the other the next.
    fn banner(&mut self, ctx: &egui::Context) -> Option<(egui::TextureHandle, &'static str)> {
        if self.banner.is_none() {
            let day = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0);
            let index = day as usize % SAMPLES.len();
            let texture = decode(SAMPLES[index].banner).map(|image| ctx.load_texture("start-banner", image, egui::TextureOptions::LINEAR));
            self.banner = Some((index, texture));
        }
        let (index, texture) = self.banner.as_ref()?;
        texture.clone().map(|t| (t, SAMPLES[*index].name))
    }

    fn recent_grid(&mut self, ui: &mut egui::Ui, recent: &[PathBuf]) -> Option<Choice> {
        let p = theme::p();
        let mut choice = None;
        heading(ui, "Recent projects");
        if recent.is_empty() {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), (ui.available_height() - 8.0).clamp(90.0, 170.0)), egui::Sense::hover());
            dashed(ui.painter(), rect, 12.0, p.line_strong);
            let glyph = Rect::from_center_size(rect.center() - egui::vec2(0.0, 16.0), Vec2::splat(26.0));
            icons::paint(ui.painter(), glyph, icons::Icon::Clock, p.text_faint);
            ui.painter().text(rect.center() + egui::vec2(0.0, 14.0), egui::Align2::CENTER_CENTER, "Projects you open and save show here", theme::strong(13.0), p.text_dim);
            ui.painter().text(rect.center() + egui::vec2(0.0, 33.0), egui::Align2::CENTER_CENTER, "each with a picture of how you left it", egui::FontId::proportional(12.0), p.text_faint);
            return None;
        }
        let width = ui.available_width();
        let columns = ((width + GAP) / (170.0 + GAP)).floor().clamp(2.0, 4.0) as usize;
        let tile = (width - GAP * (columns - 1) as f32) / columns as f32;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for row in recent.chunks(columns) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for path in row {
                        let picture = match self.pictures.get(path) {
                            Some(Picture::Shown(texture)) => Some(texture.clone()),
                            _ => None,
                        };
                        match recent_tile(ui, tile, path, picture.as_ref()) {
                            Row::Open => choice = Some(Choice::Recent(path.clone())),
                            Row::Forget => choice = Some(Choice::Forget(path.clone())),
                            Row::Nothing => {}
                        }
                    }
                });
                ui.add_space(GAP);
            }
        });
        choice
    }

    fn sample_list(&mut self, ui: &mut egui::Ui, samples: &[(&Sample, Option<PathBuf>)]) -> Option<Choice> {
        let mut choice = None;
        heading(ui, "Samples");
        for (sample, path) in samples {
            let Some(path) = path else { continue };
            let picture = match self.pictures.get(path) {
                Some(Picture::Shown(texture)) => Some(texture.clone()),
                _ => None,
            };
            if sample_row(ui, sample, picture.as_ref()).clicked() {
                choice = Some(Choice::Sample(path.clone()));
            }
            ui.add_space(10.0);
        }
        if samples.iter().all(|(_, path)| path.is_none()) {
            ui.label(egui::RichText::new("The samples are not beside this Tequila.").size(12.0).color(theme::p().text_faint));
        }
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

/// The banner, the welcome on its dark side, and the way out: true when it
/// was closed. Light on dark whatever the theme, as the picture is dark.
fn hero_section(ui: &mut egui::Ui, hero: Rect, banner: Option<&(egui::TextureHandle, &'static str)>) -> bool {
    let p = theme::p();
    let painter = ui.painter();
    let top = Rounding { nw: RADIUS, ne: RADIUS, sw: 0.0, se: 0.0 };
    match banner {
        Some((texture, _)) => cover(ui, hero, texture, top),
        None => {
            painter.rect_filled(hero, top, Color32::from_rgb(14, 15, 18));
        }
    }
    let white = |alpha: u8| Color32::from_white_alpha(alpha);
    let left = hero.left() + PAD + 4.0;
    let mark = Rect::from_min_size(egui::pos2(left, hero.top() + 26.0), Vec2::splat(26.0));
    icons::paint(painter, mark, icons::Icon::Aperture, p.accent);
    painter.text(egui::pos2(mark.right() + 10.0, mark.center().y), egui::Align2::LEFT_CENTER, "Tequila", theme::strong(16.0), white(235));

    let title_y = hero.bottom() - hero.height() * 0.42;
    painter.text(egui::pos2(left, title_y), egui::Align2::LEFT_BOTTOM, "Welcome to Tequila", theme::strong(31.0), white(250));
    painter.text(
        egui::pos2(left, title_y + 10.0),
        egui::Align2::LEFT_TOP,
        "Photoreal product shots from your CAD files and models.",
        egui::FontId::proportional(15.0),
        white(205),
    );
    // The version, as a quiet chip.
    let version = crate::update::label();
    let galley = painter.layout_no_wrap(version, egui::FontId::proportional(11.5), white(215));
    let chip = Rect::from_min_size(egui::pos2(left, title_y + 40.0), galley.size() + egui::vec2(18.0, 8.0));
    painter.rect(chip, chip.height() * 0.5, white(22), Stroke::new(1.0_f32, white(40)));
    painter.galley(chip.min + egui::vec2(9.0, 4.0), galley, Color32::PLACEHOLDER);

    if let Some((_, name)) = banner {
        painter.text(
            egui::pos2(hero.right() - PAD, hero.bottom() - 14.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{name}, a sample scene"),
            egui::FontId::proportional(11.0),
            white(120),
        );
    }

    // Closing, on a dark disc so it reads on any picture.
    let close = Rect::from_center_size(egui::pos2(hero.right() - 30.0, hero.top() + 30.0), Vec2::splat(30.0));
    let response = ui.interact(close, ui.id().with("start-close"), egui::Sense::click()).on_hover_text("Close (Esc)");
    painter.circle_filled(close.center(), 15.0, Color32::from_black_alpha(if response.hovered() { 190 } else { 120 }));
    icons::paint(painter, close.shrink(8.0), icons::Icon::Close, white(if response.hovered() { 255 } else { 210 }));
    response.clicked()
}

/// New, Open and Import as three cards across.
fn action_cards(ui: &mut egui::Ui) -> Option<Choice> {
    let mut choice = None;
    let width = (ui.available_width() - GAP * 2.0) / 3.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        let cards = [
            (theme::Hue::Blue, icons::Icon::FilePlus, "New project", "An empty studio, ready for a model", theme::keys("Ctrl+N"), Choice::New),
            (theme::Hue::Amber, icons::Icon::FolderOpen, "Open project...", "A .tequila project of yours", theme::keys("Ctrl+O"), Choice::Open),
            (theme::Hue::Green, icons::Icon::Import, "Import model...", "STEP, IGES, OBJ, glTF, STL, USD", String::new(), Choice::Import),
        ];
        for (hue, icon, title, hint, keys, chosen) in cards {
            if action_card(ui, width, hue, icon, title, hint, &keys).clicked() {
                choice = Some(chosen);
            }
        }
    });
    choice
}

fn action_card(ui: &mut egui::Ui, width: f32, hue: theme::Hue, icon: icons::Icon, title: &str, hint: &str, keys: &str) -> egui::Response {
    let p = theme::p();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 74.0), egui::Sense::click());
    let ease = ui.ctx().animate_bool_with_time(response.id.with("ease"), response.hovered(), 0.12);
    let colour = theme::hue(hue);
    let painter = ui.painter();
    painter.rect(rect, 12.0, theme::mix(p.card, p.hover, ease), Stroke::new(1.0_f32, theme::mix(p.line, colour, ease)));
    let tile = Rect::from_min_size(egui::pos2(rect.left() + 16.0, rect.center().y - 20.0), Vec2::splat(40.0));
    painter.rect_filled(tile, 10.0, colour.gamma_multiply(if p.dark { 0.2 } else { 0.14 }));
    icons::paint(painter, tile.shrink(10.0), icon, colour);
    let text_left = tile.right() + 14.0;
    painter.text(egui::pos2(text_left, rect.center().y - 9.0), egui::Align2::LEFT_CENTER, title, theme::strong(14.5), p.text);
    let room = rect.right() - text_left - 12.0;
    let hint = painter.layout(hint.to_string(), egui::FontId::proportional(12.0), p.text_dim, room);
    painter.galley(egui::pos2(text_left, rect.center().y + 3.0), hint, Color32::PLACEHOLDER);
    if !keys.is_empty() {
        painter.text(egui::pos2(rect.right() - 12.0, rect.top() + 12.0), egui::Align2::RIGHT_TOP, keys, egui::FontId::proportional(10.5), p.text_faint);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::strong(15.0)).color(theme::p().text));
    ui.add_space(8.0);
}

enum Row {
    Open,
    Forget,
    Nothing,
}

/// A recent project as a card: its picture, its name, where and when.
fn recent_tile(ui: &mut egui::Ui, width: f32, path: &Path, picture: Option<&egui::TextureHandle>) -> Row {
    let p = theme::p();
    let picture_height = (width * 0.625).round();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, picture_height + 46.0), egui::Sense::click());
    let there = path.is_file();
    let frame = Rect::from_min_size(rect.min, egui::vec2(width, picture_height));
    draw_picture(ui, frame, picture, icons::Icon::Aperture, Rounding::same(10.0));
    if !there {
        ui.painter().rect_filled(frame, 10.0, Color32::from_black_alpha(140));
    }
    ui.painter().rect_stroke(frame, 10.0, Stroke::new(if response.hovered() { 2.0_f32 } else { 1.0 }, if response.hovered() { p.accent } else { p.line }));
    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let detail = if there {
        let folder = path.parent().map(short_folder).unwrap_or_default();
        match modified_ago(path) {
            Some(age) => format!("{age}  ·  {folder}"),
            None => folder,
        }
    } else {
        "No longer there".to_string()
    };
    let painter = ui.painter();
    let line = |text: String, font: egui::FontId, colour: Color32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text, font, colour);
        job.wrap = egui::text::TextWrapping::truncate_at_width(width - 4.0);
        painter.layout_job(job)
    };
    painter.galley(egui::pos2(rect.left() + 2.0, frame.bottom() + 8.0), line(name, theme::strong(13.0), if there { p.text } else { p.text_faint }), Color32::PLACEHOLDER);
    painter.galley(egui::pos2(rect.left() + 2.0, frame.bottom() + 27.0), line(detail, egui::FontId::proportional(11.5), p.text_dim), Color32::PLACEHOLDER);

    let mut row = Row::Nothing;
    if response.hovered() {
        let forget = Rect::from_center_size(egui::pos2(frame.right() - 16.0, frame.top() + 16.0), Vec2::splat(24.0));
        let hit = ui.interact(forget, ui.id().with(("forget", path)), egui::Sense::click());
        ui.painter().circle_filled(forget.center(), 12.0, Color32::from_black_alpha(if hit.hovered() { 200 } else { 140 }));
        icons::paint(ui.painter(), forget.shrink(6.0), icons::Icon::Close, Color32::WHITE);
        if hit.on_hover_text("Take it off the list").clicked() {
            row = Row::Forget;
        }
    }
    if matches!(row, Row::Nothing) && response.on_hover_text(path.display().to_string()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        row = Row::Open;
    }
    row
}

/// A sample as a row: its picture, its name, and what it shows.
fn sample_row(ui: &mut egui::Ui, sample: &Sample, picture: Option<&egui::TextureHandle>) -> egui::Response {
    let p = theme::p();
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 78.0), egui::Sense::click());
    let ease = ui.ctx().animate_bool_with_time(response.id.with("ease"), response.hovered(), 0.12);
    if ease > 0.0 {
        ui.painter().rect_filled(rect.expand(6.0), 12.0, p.hover.gamma_multiply(ease));
    }
    let frame = Rect::from_min_size(rect.min, egui::vec2(124.0, 78.0));
    draw_picture(ui, frame, picture, icons::Icon::Gem, Rounding::same(9.0));
    ui.painter().rect_stroke(frame, 9.0, Stroke::new(1.0_f32, theme::mix(p.line, p.accent, ease)));
    let text_left = frame.right() + 12.0;
    let room = rect.right() - text_left;
    let painter = ui.painter();
    painter.text(egui::pos2(text_left, rect.top() + 12.0), egui::Align2::LEFT_CENTER, sample.name, theme::strong(13.5), p.text);
    let about = painter.layout(sample.about.to_string(), egui::FontId::proportional(11.5), p.text_dim, room);
    painter.galley(egui::pos2(text_left, rect.top() + 24.0), about, Color32::PLACEHOLDER);
    response.on_hover_text("Open it as a new project").on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// `texture` covering `frame`, cropped as a photograph is.
fn cover(ui: &egui::Ui, frame: Rect, texture: &egui::TextureHandle, rounding: Rounding) {
    let size = texture.size_vec2();
    let scale = (frame.width() / size.x).max(frame.height() / size.y);
    let shown = size * scale;
    let uv = Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2(frame.width() / shown.x, frame.height() / shown.y));
    egui::Image::new((texture.id(), frame.size())).uv(uv).rounding(rounding).paint_at(ui, frame);
}

/// A picture in `frame`, or a quiet stand-in while there is none.
fn draw_picture(ui: &egui::Ui, frame: Rect, picture: Option<&egui::TextureHandle>, stand_in: icons::Icon, rounding: Rounding) {
    let p = theme::p();
    match picture {
        Some(texture) => cover(ui, frame, texture, rounding),
        None => {
            ui.painter().rect_filled(frame, rounding, p.field);
            let glyph = Rect::from_center_size(frame.center(), Vec2::splat((frame.height() * 0.32).min(28.0)));
            icons::paint(ui.painter(), glyph, stand_in, p.text_faint);
        }
    }
}

/// A rounded rectangle's outline in dashes, for a place that is empty.
fn dashed(painter: &egui::Painter, rect: Rect, radius: f32, colour: Color32) {
    let mut points = Vec::new();
    let corners = [
        (egui::pos2(rect.right() - radius, rect.top() + radius), -std::f32::consts::FRAC_PI_2),
        (egui::pos2(rect.right() - radius, rect.bottom() - radius), 0.0),
        (egui::pos2(rect.left() + radius, rect.bottom() - radius), std::f32::consts::FRAC_PI_2),
        (egui::pos2(rect.left() + radius, rect.top() + radius), std::f32::consts::PI),
    ];
    for (centre, start) in corners {
        for i in 0..=8 {
            let a = start + std::f32::consts::FRAC_PI_2 * i as f32 / 8.0;
            points.push(centre + radius * egui::vec2(a.cos(), a.sin()));
        }
    }
    points.push(points[0]);
    painter.extend(egui::Shape::dashed_line(&points, Stroke::new(1.2_f32, colour), 6.0, 5.0));
}

/// The links, the startup switch, and a newer version when there is one:
/// true when that was clicked.
fn footer_row(ui: &mut egui::Ui, show_at_startup: &mut bool, newer: Option<crate::update::Version>) -> bool {
    let p = theme::p();
    let mut update = false;
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (icon, label, url, about) in [
            (icons::Icon::Discord, "Discord", DISCORD, "Questions, renders and news, with the people who make Tequila"),
            (icons::Icon::Github, "GitHub", GITHUB, "The source, and where to report a bug"),
            (icons::Icon::Book, "Guides", GUIDES, "How Tequila works, in depth"),
            (icons::Icon::ExternalLink, "What's new", RELEASES, "Every release and what it brought"),
        ] {
            if link(ui, icon, label).on_hover_text(about).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(crate::field::Checkbox::new(show_at_startup, "Show at startup"));
            if let Some(version) = newer {
                ui.add_space(12.0);
                let label = format!("Update to {version}");
                if icons::labelled(ui, icons::Icon::Update, &label, true)
                    .on_hover_text("A newer Tequila is out: what it brings, and installing it")
                    .clicked()
                {
                    update = true;
                }
            } else {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(crate::update::label()).size(11.5).color(p.text_faint));
            }
        });
    });
    update
}

/// A quiet link: an icon and a word, lit when pointed at.
fn link(ui: &mut egui::Ui, icon: icons::Icon, label: &str) -> egui::Response {
    let p = theme::p();
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.5), p.text_dim);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(galley.size().x + 36.0, 30.0), egui::Sense::click());
    let colour = if response.hovered() { p.text } else { p.text_dim };
    if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, p.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(egui::pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(14.0)), icon, colour);
    ui.painter().galley(egui::pos2(rect.left() + 28.0, rect.center().y - galley.size().y * 0.5), galley, colour);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
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

/// A picture saved with a project, or a banner, for the screen.
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
        assert_eq!(decoded.pixels[0], Color32::from_rgb(200, 90, 40));
        assert_eq!(decoded.pixels[512 * 319], Color32::from_rgb(20, 90, 40));
        assert!(picture_of(&crate::orchestrator::DisplayImage::default()).is_none());
    }

    /// Every banner the screen can show decodes, wide and dark on its left
    /// for the welcome to read on.
    #[test]
    fn every_banner_decodes_wide_and_dark_on_the_left() {
        for sample in &SAMPLES {
            let image = decode(sample.banner).unwrap_or_else(|| panic!("{}'s banner does not decode", sample.name));
            let [width, height] = image.size;
            assert!(width > 3 * height, "{}'s banner is {width} by {height}", sample.name);
            let left: f32 = (0..height)
                .flat_map(|y| (0..width / 3).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let c = image.pixels[y * width + x];
                    (c.r() as f32 + c.g() as f32 + c.b() as f32) / 3.0
                })
                .sum::<f32>()
                / (height * width / 3) as f32;
            assert!(left < 40.0, "{}'s banner is too light where the welcome is: {left}", sample.name);
        }
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
