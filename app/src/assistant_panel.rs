//! The Assistant's panel: a chat, as the Claude and ChatGPT apps have one.
//!
//! From the top: the chat's title with the list of past chats, New chat,
//! and the button that moves the panel into a window of its own; the
//! conversation; and the message box, with what can go into a message on
//! its left - a picture, the viewport as it is, a part or material by name -
//! and who answers on its right: how hard it thinks, which model, Send.
//!
//! What the panel draws is the chat ([`tequila_ai::chat`]), whoever is
//! answering it. What it cannot do itself - start a request, answer a
//! question, take the viewport's picture - it hands back as an
//! [`Action`] for the application.

use crate::assistant::{Action, Around, Assistant, Backend, Setup};
use crate::assistant_agents as agents;
use crate::icons::{self, Icon};
use crate::theme;
use std::collections::HashMap;
use tequila_ai::acp;
use tequila_ai::chat::{Item, Picture, Status, Summary};

/// Something of the scene a message can name: the selection, a part, a
/// material.
#[derive(Clone, Debug, PartialEq)]
pub struct Thing {
    /// Its name, as it is written after the `@`.
    pub label: String,
    /// What it is, in a word, for the list.
    pub kind: String,
    /// What the model is told it is: "Grille is part 12".
    pub context: String,
}

/// What the panel keeps between frames.
#[derive(Default)]
pub struct State {
    /// The list of past chats is showing, in place of the conversation.
    history: bool,
    /// The past chats, and when they were read from disk.
    past: Option<(std::time::Instant, Vec<Summary>)>,
    /// A chat asked to be deleted, waiting for a second click.
    deleting: Option<String>,
    /// Pictures attached to the message being written.
    pub(crate) attached: Vec<Picture>,
    /// Things named in the message being written.
    mentions: Vec<Thing>,
    /// The list of things to name is showing, opened by the button.
    naming: bool,
    /// Pictures as the graphics card holds them, by [`Picture::id`].
    textures: HashMap<u64, egui::TextureHandle>,
    /// Where the panel is in the window, for a file dropped on it.
    pub(crate) rect: Option<egui::Rect>,
    /// What an install or a sign-in last said.
    notice: Option<String>,
}

/// The longest a picture is drawn in the conversation, in points.
const PICTURE: f32 = 220.0;

/// `0:42`, `12:05`, `1:02:03`.
pub(crate) fn clock(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// `8 s`, `2 min 5 s`.
pub(crate) fn spoken(seconds: f32) -> String {
    let whole = seconds.round().max(1.0) as u64;
    match (whole / 60, whole % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

/// When a chat was last touched, as the list says it.
fn day(updated: u64, now: u64) -> &'static str {
    match now.saturating_sub(updated) / 86_400 {
        0 => "Today",
        1 => "Yesterday",
        2..=6 => "This week",
        _ => "Earlier",
    }
}

/// A PNG as a picture egui can draw.
fn decode(png_bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixels = &buffer[..info.buffer_size()];
    let size = [info.width as usize, info.height as usize];
    match info.color_type {
        png::ColorType::Rgba => Some(egui::ColorImage::from_rgba_unmultiplied(size, pixels)),
        png::ColorType::GrayscaleAlpha => {
            let rgba: Vec<u8> = pixels.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect();
            Some(egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
        }
        _ => None,
    }
}

/// One piece of a line of the reply: plain, bold, or code.
#[derive(Debug, PartialEq)]
enum Span<'a> {
    Plain(&'a str),
    Bold(&'a str),
    Code(&'a str),
}

/// A line of a reply split at its `**bold**` and `` `code` ``, which is as
/// much of Markdown as a chat about a scene uses within a line. A mark
/// that is never closed is left as it was written.
fn spans(line: &str) -> Vec<Span<'_>> {
    let mut out = Vec::new();
    let mut rest = line;
    loop {
        let bold = rest.find("**");
        let code = rest.find('`');
        let (at, mark, bolded) = match (bold, code) {
            (Some(b), Some(c)) if b < c => (b, "**", true),
            (_, Some(c)) => (c, "`", false),
            (Some(b), None) => (b, "**", true),
            (None, None) => break,
        };
        let after = &rest[at + mark.len()..];
        let Some(end) = after.find(mark).filter(|end| *end > 0) else { break };
        if at > 0 {
            out.push(Span::Plain(&rest[..at]));
        }
        out.push(if bolded { Span::Bold(&after[..end]) } else { Span::Code(&after[..end]) });
        rest = &after[end + mark.len()..];
    }
    if !rest.is_empty() {
        out.push(Span::Plain(rest));
    }
    out
}

/// A reply, drawn: paragraphs, headings, lists and blocks of code, with
/// bold and code within a line.
fn reply(ui: &mut egui::Ui, text: &str) {
    let p = theme::p();
    let size = 13.5;
    let line_of = |ui: &mut egui::Ui, line: &str, heading: bool| {
        let mut job = egui::text::LayoutJob::default();
        let plain = egui::TextFormat { font_id: if heading { theme::strong(size + 1.0) } else { egui::FontId::proportional(size) }, color: p.text, ..Default::default() };
        for span in spans(line) {
            match span {
                Span::Plain(text) => job.append(text, 0.0, plain.clone()),
                Span::Bold(text) => job.append(text, 0.0, egui::TextFormat { font_id: theme::strong(size), ..plain.clone() }),
                Span::Code(text) => job.append(text, 0.0, egui::TextFormat { font_id: egui::FontId::monospace(size - 1.0), background: p.field, ..plain.clone() }),
            }
        }
        ui.add(egui::Label::new(job).wrap());
    };
    let mut code: Option<Vec<&str>> = None;
    let mut gap = false;
    for line in text.trim().lines() {
        if line.trim_start().starts_with("```") {
            match code.take() {
                Some(lines) => {
                    egui::Frame::none().fill(p.field).rounding(6.0).inner_margin(8.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(egui::Label::new(egui::RichText::new(lines.join("\n")).monospace().size(size - 1.5).color(p.text)).wrap());
                    });
                }
                None => code = Some(Vec::new()),
            }
            continue;
        }
        if let Some(lines) = &mut code {
            lines.push(line);
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            gap = true;
            continue;
        }
        if std::mem::take(&mut gap) {
            ui.add_space(6.0);
        }
        let indent = (line.len() - line.trim_start().len()) as f32 * 4.0;
        if let Some(heading) = trimmed.strip_prefix('#').map(|h| h.trim_start_matches('#').trim_start()) {
            ui.add_space(2.0);
            line_of(ui, heading, true);
        } else if let Some(item) = ["- ", "* ", "+ "].iter().find_map(|mark| trimmed.strip_prefix(mark)) {
            ui.horizontal_top(|ui| {
                ui.add_space(4.0 + indent);
                ui.label(egui::RichText::new("\u{2022}").size(size).color(p.text_dim));
                ui.vertical(|ui| line_of(ui, item, false));
            });
        } else {
            ui.horizontal_top(|ui| {
                ui.add_space(indent);
                ui.vertical(|ui| line_of(ui, trimmed, false));
            });
        }
    }
    // A block of code still being written.
    if let Some(lines) = code {
        egui::Frame::none().fill(p.field).rounding(6.0).inner_margin(8.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(egui::RichText::new(lines.join("\n")).monospace().size(size - 1.5).color(p.text)).wrap());
        });
    }
}

/// Reasoning, as the panel shows it: small and dimmed, beside the answer
/// rather than part of it.
fn reasoning(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text.trim()).small().italics().color(theme::dim())).wrap());
}

/// A small button that is only its icon, for the header and the message
/// box: quiet until the pointer is on it.
fn quiet(ui: &mut egui::Ui, icon: Icon, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = theme::p();
        let enabled = ui.is_enabled();
        let ease = ui.ctx().animate_bool_with_time(response.id.with("ease"), enabled && response.hovered(), 0.1);
        let fill = if on { p.control } else { theme::mix(p.panel, p.hover, ease) };
        ui.painter().rect_filled(rect, 6.0, fill);
        let colour = if enabled { theme::mix(p.text_dim, p.text, ease.max(if on { 1.0 } else { 0.0 })) } else { p.text_faint };
        icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0)), icon, colour);
    }
    response
}

/// A choice shown as a small outlined word with a menu behind it: the
/// model, how hard it thinks.
fn pill<R>(ui: &mut egui::Ui, label: &str, menu: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let p = theme::p();
    let button = egui::Button::new(egui::RichText::new(format!("{label} \u{23f7}")).size(11.5).color(p.text_dim)).fill(p.panel).stroke(egui::Stroke::new(1.0_f32, p.line_strong)).rounding(6.0);
    // Closer set than a button in a form: the row has five things in it.
    let padding = std::mem::replace(&mut ui.spacing_mut().button_padding, egui::vec2(6.0, 3.0));
    let shown = egui::menu::menu_custom_button(ui, button, menu).inner;
    ui.spacing_mut().button_padding = padding;
    shown
}

impl State {
    /// Show the list of past chats, or go back to the conversation.
    pub(crate) fn toggle_history(&mut self) {
        self.show_history(!self.history);
    }

    pub(crate) fn show_history(&mut self, show: bool) {
        self.history = show;
        self.past = None;
    }

    fn texture(&mut self, ctx: &egui::Context, picture: &Picture) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.textures.get(&picture.id) {
            return Some(texture.clone());
        }
        let image = decode(&picture.png)?;
        let texture = ctx.load_texture(format!("assistant-picture-{}", picture.id), image, egui::TextureOptions::LINEAR);
        self.textures.insert(picture.id, texture.clone());
        Some(texture)
    }

    /// Pictures in a row, each no longer than `longest`.
    fn pictures(&mut self, ui: &mut egui::Ui, pictures: &[Picture], longest: f32) {
        for picture in pictures {
            if let Some(texture) = self.texture(ui.ctx(), picture) {
                let size = texture.size_vec2();
                let scale = (longest / size.x.max(size.y).max(1.0)).min(1.0);
                ui.add(egui::Image::new(&texture).fit_to_exact_size(size * scale).rounding(6.0));
            }
        }
    }

    /// Forget drawn pictures no chat on screen has any more.
    fn forget_textures(&mut self, kept: impl Iterator<Item = u64>) {
        let kept: std::collections::HashSet<u64> = kept.collect();
        self.textures.retain(|id, _| kept.contains(id));
    }
}

impl Assistant {
    /// What the model picker says is answering.
    pub(crate) fn answering(&self) -> String {
        let name = match self.backend() {
            Some(Backend::Agent(kind)) => match self.agent.as_ref().and_then(|(_, agent)| agent.setting("model")) {
                // The model's own name, with whose it is where it does not
                // say: "Sonnet" as "Claude Sonnet".
                Some(model) if model.current_name().to_lowercase().contains(&kind.name().to_lowercase()) => model.current_name().to_string(),
                Some(model) => format!("{} {}", kind.name(), model.current_name()),
                None => kind.name().to_string(),
            },
            Some(Backend::Provider(provider)) if provider.model.is_empty() => provider.name,
            Some(Backend::Provider(provider)) => provider.model,
            None => "Choose an AI".into(),
        };
        // Short enough to leave the row's other buttons their room.
        if name.chars().count() > 22 {
            format!("{}\u{2026}", name.chars().take(21).collect::<String>().trim_end())
        } else {
            name
        }
    }

    /// Attach a picture to the message being written: shrunk to what a
    /// model is sent, and kept as a PNG.
    pub fn attach(&mut self, png: &[u8]) {
        match crate::assistant::prepare_image(png) {
            Some(png) => self.panel.attached.push(Picture::new(png)),
            None => self.panel.notice = Some("That picture could not be read.".into()),
        }
    }

    /// Attach a picture from a file: any kind Tequila reads as a texture.
    pub fn attach_file(&mut self, path: &std::path::Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Ok(image) = tequila_texture::Image::load_raw(path) else {
            self.panel.notice = Some(format!("{name} could not be read as a picture."));
            return;
        };
        let (width, height, channels) = (image.width, image.height, image.channels.max(1));
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut rgba = Vec::with_capacity(width * height * 4);
        for pixel in image.data.chunks_exact(channels) {
            match channels {
                1 => rgba.extend([byte(pixel[0]), byte(pixel[0]), byte(pixel[0]), 255]),
                2 => rgba.extend([byte(pixel[0]), byte(pixel[0]), byte(pixel[0]), byte(pixel[1])]),
                3 => rgba.extend([byte(pixel[0]), byte(pixel[1]), byte(pixel[2]), 255]),
                _ => rgba.extend([byte(pixel[0]), byte(pixel[1]), byte(pixel[2]), byte(pixel[3])]),
            }
        }
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, width as u32, height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let Some(mut writer) = encoder.write_header().ok() else { return };
            if writer.write_image_data(&rgba).is_err() {
                return;
            }
        }
        self.attach(&png);
        self.open = true;
        self.focus_input = true;
    }

    /// Whether a picture file dropped at `at` is for the message box.
    pub fn takes_drop(&self, at: Option<egui::Pos2>, path: &std::path::Path) -> bool {
        let picture = path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"].contains(&e.to_ascii_lowercase().as_str()));
        picture && self.open && self.panel.rect.zip(at).is_some_and(|(rect, at)| rect.contains(at))
    }

    /// The panel. `things` lists what a message can name with `@`, asked
    /// for only while that list is open.
    pub fn show(&mut self, ui: &mut egui::Ui, can_undo_turn: bool, around: &Around, things: &dyn Fn() -> Vec<Thing>) -> Action {
        let mut action = Action::None;
        self.panel.rect = Some(ui.max_rect());
        let p = theme::p();
        let working = self.is_working();

        // An agent chosen and not yet started is started as the panel
        // opens: its models are what the picker lists, and what it still
        // needs - installing, signing in - is said before anything is asked.
        if let (Some(Backend::Agent(kind)), None, None, false, false) = (self.backend(), &self.agent, &self.setup, working, self.rested) {
            self.start_agent(kind, around);
        }

        egui::TopBottomPanel::top("assistant-header").frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(8.0, 6.0))).show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if quiet(ui, Icon::History, self.panel.history).on_hover_text("Past chats").clicked() {
                    self.panel.toggle_history();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if quiet(ui, Icon::Close, false).on_hover_text("Close the Assistant").clicked() {
                        action = Action::Close;
                    }
                    let (icon, hint) = if self.detached { (Icon::Download, "Back beside the viewport") } else { (Icon::ExternalLink, "Open in its own window") };
                    if quiet(ui, icon, false).on_hover_text(hint).clicked() {
                        action = Action::Detach(!self.detached);
                    }
                    if quiet(ui, Icon::Pencil, false).on_hover_text("New chat").clicked() {
                        action = Action::NewChat;
                        self.panel.history = false;
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let title = if self.panel.history {
                            "Chats".to_string()
                        } else if self.chat.title.is_empty() {
                            "New chat".to_string()
                        } else {
                            self.chat.title.clone()
                        };
                        ui.add(egui::Label::new(egui::RichText::new(title).font(theme::strong(13.5)).color(p.text)).truncate());
                    });
                });
            });
        });

        if !self.panel.history {
            egui::TopBottomPanel::bottom("assistant-prompt").frame(egui::Frame::none().inner_margin(egui::Margin { left: 10.0, right: 10.0, top: 4.0, bottom: 10.0 })).show_inside(ui, |ui| {
                self.status_row(ui, can_undo_turn, &mut action);
                self.message_box(ui, around, things, &mut action);
            });
        }

        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(10.0, 0.0))).show_inside(ui, |ui| {
            if self.panel.history {
                self.history(ui, &mut action);
            } else {
                self.conversation(ui, around, &mut action);
            }
        });
        action
    }

    /// The line above the message box: what it is doing and for how long,
    /// or the way to take the last request back.
    fn status_row(&mut self, ui: &mut egui::Ui, can_undo_turn: bool, action: &mut Action) {
        let working = self.is_working();
        if !working && !can_undo_turn {
            return;
        }
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match self.asked.filter(|_| working) {
                    Some(asked) => {
                        ui.label(egui::RichText::new(clock(asked.elapsed())).monospace().small().color(theme::dim()))
                            .on_hover_text("How long it has been working on this request");
                    }
                    None if can_undo_turn => {
                        if ui.add(egui::Button::new(egui::RichText::new("Undo request").small()).frame(false)).on_hover_text("Take back everything the last request changed").clicked() {
                            *action = Action::UndoTurn;
                        }
                    }
                    None => {}
                }
                if working {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if !self.is_asking() {
                            ui.add(egui::Spinner::new().size(12.0));
                        }
                        ui.add(egui::Label::new(egui::RichText::new(self.status_line()).small().color(theme::dim())).truncate());
                    });
                }
            });
        });
        ui.add_space(4.0);
    }

    /// The message box: what is attached, the words, and the row under
    /// them - what can be added on the left, who answers on the right.
    fn message_box(&mut self, ui: &mut egui::Ui, around: &Around, things: &dyn Fn() -> Vec<Thing>, action: &mut Action) {
        let p = theme::p();
        let working = self.is_working();
        let id = ui.make_persistent_id("assistant-input");
        // Enter sends; Shift+Enter is a new line.
        let enter = ui.memory(|m| m.has_focus(id)) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let mut send = false;
        egui::Frame::none().fill(p.field).rounding(12.0).stroke(egui::Stroke::new(1.0_f32, p.line_strong)).inner_margin(egui::Margin::symmetric(8.0, 8.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(notice) = self.panel.notice.clone() {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(egui::RichText::new(notice).small().color(p.danger)).wrap());
                    if ui.small_button("OK").clicked() {
                        self.panel.notice = None;
                    }
                });
            }
            // What is attached, each with a way to take it off again.
            if !self.panel.attached.is_empty() {
                let mut removed = None;
                ui.horizontal_wrapped(|ui| {
                    for (index, picture) in self.panel.attached.clone().iter().enumerate() {
                        let Some(texture) = self.panel.texture(ui.ctx(), picture) else { continue };
                        let size = texture.size_vec2();
                        let scale = 44.0 / size.y.max(1.0);
                        let shown = ui.add(egui::Image::new(&texture).fit_to_exact_size(size * scale).rounding(6.0).sense(egui::Sense::click())).on_hover_text("Click to take it off the message");
                        if shown.clicked() {
                            removed = Some(index);
                        }
                    }
                });
                if let Some(index) = removed {
                    self.panel.attached.remove(index);
                }
                ui.add_space(4.0);
            }
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.input)
                    .id(id)
                    .frame(false)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text("Ask for anything"),
            );
            if std::mem::take(&mut self.focus_input) {
                response.request_focus();
            }
            // Naming a part or a material: the list opens at an `@` being
            // written, or from the button.
            let typing = self.input.rfind('@').map(|at| self.input[at + 1..].to_string()).filter(|word| word.len() < 30 && !word.contains('\n') && !self.panel.mentions.iter().any(|m| word.starts_with(&m.label)));
            if typing.is_some() || self.panel.naming {
                let filter = typing.clone().unwrap_or_default().to_lowercase();
                let matching: Vec<Thing> = things().into_iter().filter(|thing| filter.is_empty() || thing.label.to_lowercase().contains(&filter)).take(8).collect();
                if !matching.is_empty() {
                    egui::Frame::none().fill(p.raised).rounding(8.0).stroke(egui::Stroke::new(1.0_f32, p.line)).inner_margin(4.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for thing in matching {
                            let row = ui.horizontal(|ui| {
                                let chosen = ui.add(egui::Button::new(egui::RichText::new(&thing.label).size(12.5)).frame(false));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(egui::RichText::new(&thing.kind).small().color(theme::dim())));
                                chosen.clicked()
                            });
                            if row.inner {
                                // The word being written gives way to the name.
                                if let (Some(_), Some(at)) = (&typing, self.input.rfind('@')) {
                                    self.input.truncate(at);
                                }
                                if !self.input.is_empty() && !self.input.ends_with([' ', '\n']) {
                                    self.input.push(' ');
                                }
                                self.input.push_str(&format!("@{} ", thing.label));
                                self.panel.mentions.push(thing);
                                self.panel.naming = false;
                                self.focus_input = true;
                            }
                        }
                    });
                } else if self.panel.naming {
                    ui.label(egui::RichText::new("Nothing in the scene to name yet.").small().color(theme::dim()));
                }
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if quiet(ui, Icon::Plus, false).on_hover_text("Attach a picture (or drop one here)").clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Pictures", &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"]).pick_file() {
                        self.attach_file(&path);
                    }
                }
                if quiet(ui, Icon::Camera, false).on_hover_text("Send the viewport as it is now").clicked() {
                    *action = Action::SendViewport;
                }
                let name = ui.add(egui::Button::new(egui::RichText::new("@").size(14.0).color(p.text_dim)).frame(false).min_size(egui::vec2(26.0, 26.0))).on_hover_text("Name a part, a material or the selection");
                if name.clicked() {
                    self.panel.naming = !self.panel.naming;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Send, or Stop while it works: one round button.
                    let ready = !self.input.trim().is_empty() || !self.panel.attached.is_empty();
                    let (rect, button) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
                    let live = working || ready;
                    ui.painter().circle_filled(rect.center(), 14.0, if live { p.accent } else { p.control });
                    if working {
                        ui.painter().rect_filled(egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(9.0)), 2.0, p.on_accent);
                    } else {
                        icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0)), Icon::ArrowUp, if ready { p.on_accent } else { p.text_faint });
                    }
                    let button = button.on_hover_text(if working { "Stop" } else { "Send (Enter)" });
                    if working && button.clicked() {
                        *action = Action::Stop;
                    } else if !working && ready && (button.clicked() || enter) {
                        send = true;
                    }
                    self.model_picker(ui, around);
                    // How hard it thinks, where the row has room for it.
                    if ui.available_width() > 76.0 {
                        self.thinking_picker(ui);
                    }
                });
            });
        });
        if send {
            let text = std::mem::take(&mut self.input).trim().to_string();
            // What the model is told of the things the message names.
            let named: Vec<String> = std::mem::take(&mut self.panel.mentions).into_iter().filter(|thing| text.contains(&format!("@{}", thing.label))).map(|thing| thing.context).collect();
            let context = if named.is_empty() { String::new() } else { format!("(In this scene: {}.)", named.join("; ")) };
            *action = Action::Ask { text, context, pictures: std::mem::take(&mut self.panel.attached) };
            self.panel.naming = false;
        }
    }

    /// Which model answers: the agent's own models while it runs, each
    /// connected provider, and the way to connect more.
    fn model_picker(&mut self, ui: &mut egui::Ui, around: &Around) {
        let label = self.answering();
        let working = self.is_working();
        let mut chosen: Option<(Backend, Option<(String, String)>)> = None;
        let mut connections = false;
        pill(ui, &label, |ui| {
            ui.set_min_width(200.0);
            let current = self.backend();
            for kind in agents::Kind::ALL {
                ui.label(egui::RichText::new(kind.name()).small().color(theme::dim()));
                let running = self.agent.as_ref().filter(|(running, _)| *running == kind).and_then(|(_, agent)| agent.setting("model"));
                match running {
                    Some(model) => {
                        let mut group = "";
                        for choice in &model.choices {
                            if choice.group != group && !choice.group.is_empty() {
                                ui.label(egui::RichText::new(&choice.group).small().color(theme::dim()));
                            }
                            group = &choice.group;
                            let on = current == Some(Backend::Agent(kind)) && model.current == choice.value;
                            let row = ui.selectable_label(on, &choice.name);
                            let row = if choice.description.is_empty() { row } else { row.on_hover_text(&choice.description) };
                            if row.clicked() {
                                chosen = Some((Backend::Agent(kind), Some((model.id.clone(), choice.value.clone()))));
                                ui.close_menu();
                            }
                        }
                    }
                    None => {
                        if ui.selectable_label(current == Some(Backend::Agent(kind)), kind.name()).clicked() {
                            chosen = Some((Backend::Agent(kind), None));
                            ui.close_menu();
                        }
                    }
                }
            }
            if !self.settings.providers.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new("API keys and this computer").small().color(theme::dim()));
                for provider in &self.settings.providers {
                    let on = current.as_ref().is_some_and(|c| c.id() == provider.id);
                    if ui.selectable_label(on, format!("{} \u{b7} {}", provider.name, provider.model)).clicked() {
                        chosen = Some((Backend::Provider(provider.clone()), None));
                        ui.close_menu();
                    }
                }
            }
            ui.separator();
            if ui.button("Connections\u{2026}").clicked() {
                connections = true;
                ui.close_menu();
            }
        });
        if connections {
            self.set_providers_open(true);
        }
        if let Some((backend, setting)) = chosen.filter(|_| !working) {
            self.choose(&backend);
            if let (Backend::Agent(kind), Some((setting, value))) = (&backend, setting) {
                self.settings.choices.insert(format!("{}/model", kind.id()), value.clone());
                self.save_settings();
                if let Some((_, agent)) = &mut self.agent {
                    agent.set(&setting, &value);
                }
            }
            if let (Backend::Agent(kind), None) = (&backend, &self.agent) {
                self.start_agent(*kind, around);
            }
        }
    }

    /// How hard the model thinks, where the one answering offers that.
    fn thinking_picker(&mut self, ui: &mut egui::Ui) {
        let Some((kind, agent)) = &self.agent else { return };
        let Some(level) = agent.setting("thought_level").cloned() else { return };
        let kind = *kind;
        let mut chosen = None;
        let shown = pill(ui, level.current_name(), |ui| {
            ui.label(egui::RichText::new("How hard it thinks").small().color(theme::dim()));
            for choice in &level.choices {
                let row = ui.selectable_label(choice.value == level.current, &choice.name);
                let row = if choice.description.is_empty() { row } else { row.on_hover_text(&choice.description) };
                if row.clicked() {
                    chosen = Some(choice.value.clone());
                    ui.close_menu();
                }
            }
        });
        let _ = shown;
        if let Some(value) = chosen {
            self.settings.choices.insert(format!("{}/thought_level", kind.id()), value.clone());
            self.save_settings();
            if let Some((_, agent)) = &mut self.agent {
                agent.set(&level.id, &value);
            }
        }
    }

    /// The list of past chats, newest first, under the day they were last
    /// touched.
    fn history(&mut self, ui: &mut egui::Ui, action: &mut Action) {
        let p = theme::p();
        let stale = self.panel.past.as_ref().is_none_or(|(at, _)| at.elapsed() > std::time::Duration::from_secs(2));
        if stale {
            let chats = self.store.as_ref().map(|store| store.list()).unwrap_or_default();
            self.panel.past = Some((std::time::Instant::now(), chats));
        }
        let chats = self.panel.past.as_ref().map(|(_, chats)| chats.clone()).unwrap_or_default();
        let now = tequila_ai::chat::now();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if chats.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("No chats yet").font(theme::strong(13.5)));
                    ui.label(egui::RichText::new("What you ask the Assistant is kept here, to open again and carry on.").small().color(theme::dim()));
                });
                return;
            }
            let mut heading = "";
            for chat in &chats {
                let when = day(chat.updated, now);
                if when != heading {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(when).small().color(theme::dim()));
                    heading = when;
                }
                let open = chat.id == self.chat.id;
                let row = egui::Frame::none().fill(if open { p.control } else { p.panel }).rounding(6.0).inner_margin(egui::Margin::symmetric(8.0, 5.0)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let deleting = self.panel.deleting.as_deref() == Some(chat.id.as_str());
                            if deleting {
                                if ui.add(egui::Button::new(egui::RichText::new("Delete").small().color(p.danger)).frame(false)).clicked() {
                                    *action = Action::DeleteChat(chat.id.clone());
                                    self.panel.deleting = None;
                                    self.panel.past = None;
                                }
                            } else if quiet(ui, Icon::Trash, false).on_hover_text("Delete this chat").clicked() {
                                self.panel.deleting = Some(chat.id.clone());
                            }
                            let by = agents::Kind::from_id(&chat.backend).map(|kind| kind.name().to_string()).unwrap_or_else(|| {
                                self.settings.providers.iter().find(|p| p.id == chat.backend).map(|p| p.name.clone()).unwrap_or_default()
                            });
                            ui.label(egui::RichText::new(by).small().color(theme::dim()));
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                let title = if chat.title.is_empty() { "Untitled" } else { chat.title.as_str() };
                                ui.add(egui::Label::new(egui::RichText::new(title).size(13.0)).truncate().sense(egui::Sense::click())).clicked()
                            })
                            .inner
                        })
                        .inner
                    })
                    .inner
                });
                let clicked = row.inner || row.response.interact(egui::Sense::click()).clicked();
                if clicked && !matches!(action, Action::DeleteChat(_)) {
                    *action = Action::OpenChat(chat.id.clone());
                    self.panel.history = false;
                }
            }
        });
    }

    /// What stands between the person and an agent that answers: a card
    /// that says so, with the button that deals with it.
    fn setup_card(&mut self, ui: &mut egui::Ui, around: &Around) {
        let p = theme::p();
        let Some(Backend::Agent(kind)) = self.backend() else { return };
        let Some(setup) = &self.setup else { return };
        let mut install = false;
        let mut retry = false;
        let mut key = false;
        let mut sign_in: Option<acp::SignIn> = None;
        let mut authenticate: Option<String> = None;
        egui::Frame::none().fill(p.card).rounding(10.0).stroke(egui::Stroke::new(1.0_f32, p.line)).inner_margin(12.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let say = |ui: &mut egui::Ui, text: &str| {
                ui.add(egui::Label::new(egui::RichText::new(text).size(12.5).color(p.text_dim)).wrap());
            };
            match setup {
                Setup::Missing { node: true } => {
                    ui.label(egui::RichText::new(format!("{} is not on this computer yet", kind.name())).font(theme::strong(13.5)));
                    say(ui, &kind.install_says());
                    ui.add_space(6.0);
                    install = ui.button("Install").clicked();
                }
                Setup::Missing { node: false } => {
                    ui.label(egui::RichText::new(format!("{} needs Node.js", kind.name())).font(theme::strong(13.5)));
                    say(ui, &format!("The program that runs {} here is installed with Node.js, which is not on this computer. Install Node.js, then try again.", kind.name()));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Get Node.js").clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab("https://nodejs.org"));
                        }
                        retry = ui.button("Try again").clicked();
                    });
                }
                Setup::Installing(_) => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(format!("Installing {}\u{2026} this can take a few minutes.", kind.name()));
                    });
                }
                Setup::SignIn { ways, subscription } => {
                    ui.label(egui::RichText::new(format!("{} needs an account to answer", kind.name())).font(theme::strong(13.5)));
                    match kind {
                        agents::Kind::Claude if !self.settings.claude_subscription => {
                            say(ui, "Here Claude is paid for by use, with an Anthropic API key or Console account. That is separate from a Claude subscription.");
                            if *subscription {
                                say(ui, "A Claude subscription is signed in on this computer. Tequila leaves it alone: Anthropic's terms do not plainly allow an application that is not theirs to use one.");
                            }
                        }
                        agents::Kind::Claude => say(ui, "Signing in happens in its own program, in a terminal window. Tequila never sees the login."),
                        agents::Kind::ChatGpt => say(ui, "Sign in with your ChatGPT account on OpenAI's own page, which opens in your browser, or connect an OpenAI API key, billed by use. Tequila never sees the login."),
                        agents::Kind::Gemini => say(ui, "Sign in with your Google account on Google's own page, which opens in your browser, or connect a Gemini API key, billed by use. Tequila never sees the login."),
                    }
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        // A key is signed in with by connecting it, not by a
                        // button that has nothing to hand over.
                        for way in ways.iter().filter(|way| !way.id.contains("api-key")) {
                            let button = ui.button(&way.name);
                            let button = if way.description.is_empty() { button } else { button.on_hover_text(&way.description) };
                            if button.clicked() {
                                match way.terminal {
                                    Some(_) => sign_in = Some(way.clone()),
                                    None => authenticate = Some(way.id.clone()),
                                }
                            }
                        }
                        key = ui.button("Add an API key\u{2026}").clicked();
                        retry = ui.button("Try again").on_hover_text("After signing in or adding a key").clicked();
                    });
                }
                Setup::SigningIn => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(egui::RichText::new(format!("Signing in to {}", kind.name())).font(theme::strong(13.5)));
                    });
                    say(ui, "Finish in the browser page that opened. This goes away by itself when it is done.");
                    ui.add_space(6.0);
                    retry = ui.button("Start over").clicked();
                }
                Setup::Failed(error) => {
                    ui.add(egui::Label::new(egui::RichText::new(error).color(p.danger)).wrap());
                    ui.add_space(6.0);
                    retry = ui.button("Try again").clicked();
                }
            }
        });
        if install {
            self.install(kind);
        }
        if let Some(way) = sign_in {
            self.sign_in(&way);
        }
        if let Some(method) = authenticate {
            self.authenticate(&method);
        }
        if key {
            self.add_provider(kind.key_preset());
        }
        if retry {
            self.retry(around);
        }
    }

    /// The conversation: each thing said and done, in order, then the
    /// reply on its way.
    fn conversation(&mut self, ui: &mut egui::Ui, around: &Around, action: &mut Action) {
        let p = theme::p();
        let working = self.is_working();
        let items = self.chat.items.clone();
        let thinking = self.chat.is_thinking() && working;
        self.panel.forget_textures(
            items
                .iter()
                .flat_map(|item| match item {
                    Item::You { pictures, .. } => pictures.clone(),
                    Item::Step(step) => step.pictures.clone(),
                    _ => Vec::new(),
                })
                .chain(self.panel.attached.clone())
                .map(|picture| picture.id),
        );
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            if items.is_empty() {
                ui.add_space(28.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("What should we make?").font(theme::strong(16.0)));
                    ui.add_space(2.0);
                    ui.add(egui::Label::new(egui::RichText::new("Describe it, and the Assistant sets it up in the scene: materials, lighting, camera. Every step can be undone.").size(12.5).color(theme::dim())).wrap());
                });
                ui.add_space(12.0);
                if self.backend().is_none() {
                    ui.vertical_centered(|ui| {
                        for kind in agents::Kind::ALL {
                            if ui.button(format!("Use {}", kind.name())).clicked() {
                                self.choose(&Backend::Agent(kind));
                            }
                        }
                        if ui.button("Connect an API key or a model on this computer\u{2026}").clicked() {
                            self.set_providers_open(true);
                        }
                    });
                }
            }
            let mut previous_step = false;
            for (index, item) in items.iter().enumerate() {
                let step = matches!(item, Item::Step(_));
                if !(step && previous_step) {
                    ui.add_space(6.0);
                }
                previous_step = step;
                match item {
                    Item::You { text, pictures } => {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            egui::Frame::none().fill(p.card).rounding(12.0).inner_margin(egui::Margin::symmetric(10.0, 7.0)).show(ui, |ui| {
                                ui.set_max_width((ui.available_width() * 0.88).max(120.0));
                                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                    if !text.is_empty() {
                                        ui.add(egui::Label::new(egui::RichText::new(text).size(13.5).color(p.text)).wrap());
                                    }
                                    if !pictures.is_empty() {
                                        ui.horizontal_wrapped(|ui| self.panel.pictures(ui, pictures, 96.0));
                                    }
                                });
                            });
                        });
                    }
                    Item::Said(text) => reply(ui, text),
                    Item::Thought { text, seconds } => {
                        let live = thinking && index + 1 == items.len();
                        let title = if live { "Thinking\u{2026}".to_string() } else { format!("Thought for {}", spoken(*seconds)) };
                        let title = egui::RichText::new(title).small().color(theme::dim());
                        egui::CollapsingHeader::new(title).id_salt(("assistant-thought", index)).default_open(false).show(ui, |ui| reasoning(ui, text));
                    }
                    Item::Step(step) => {
                        let row = ui.horizontal(|ui| {
                            // A rule down the left ties a run of steps together.
                            let (rule, _) = ui.allocate_exact_size(egui::vec2(2.0, 18.0), egui::Sense::hover());
                            ui.painter().rect_filled(rule.expand2(egui::vec2(0.0, 2.0)), 1.0, p.line_strong);
                            let (mark, colour) = match step.status {
                                Status::Running => (None, theme::dim()),
                                Status::Done => (Some(Icon::Check), theme::dim()),
                                Status::Failed => (Some(Icon::Close), p.danger),
                            };
                            match mark {
                                Some(icon) => {
                                    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(13.0), egui::Sense::hover());
                                    icons::paint(ui.painter(), rect, icon, colour);
                                }
                                None => {
                                    ui.add(egui::Spinner::new().size(11.0));
                                }
                            }
                            ui.add(egui::Label::new(egui::RichText::new(&step.title).size(12.0).color(colour)).truncate())
                        });
                        if !step.detail.is_empty() {
                            row.inner.on_hover_text(&step.detail);
                        }
                        if !step.pictures.is_empty() {
                            ui.horizontal(|ui| {
                                ui.add_space(22.0);
                                ui.vertical(|ui| self.panel.pictures(ui, &step.pictures, PICTURE));
                            });
                        }
                    }
                    Item::Asked(asked) => {
                        let amber = p.busy;
                        egui::Frame::none().fill(theme::mix(p.panel, amber, 0.12)).rounding(10.0).stroke(egui::Stroke::new(1.0_f32, theme::mix(p.panel, amber, 0.55))).inner_margin(10.0).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(egui::RichText::new(&asked.title).font(theme::strong(13.5)).color(p.text));
                            if !asked.detail.is_empty() {
                                ui.add(egui::Label::new(egui::RichText::new(&asked.detail).size(12.0).color(p.text_dim)).wrap());
                            }
                            ui.add_space(4.0);
                            match asked.answer {
                                None => {
                                    ui.horizontal(|ui| {
                                        if ui.button(egui::RichText::new("Allow").strong()).clicked() {
                                            *action = Action::Answer { question: asked.id, allow: true };
                                        }
                                        if ui.button("Deny").clicked() {
                                            *action = Action::Answer { question: asked.id, allow: false };
                                        }
                                    });
                                }
                                Some(true) => {
                                    ui.label(egui::RichText::new("Allowed").small().color(p.success));
                                }
                                Some(false) => {
                                    ui.label(egui::RichText::new("Not allowed").small().color(theme::dim()));
                                }
                            }
                        });
                    }
                    Item::Problem(text) => {
                        ui.add(egui::Label::new(egui::RichText::new(text).color(p.danger)).wrap());
                    }
                    Item::Note(text) => {
                        ui.add(egui::Label::new(egui::RichText::new(text).small().italics().color(theme::dim())).wrap());
                    }
                }
            }
            // The reply on its way from Tequila's own loop: its reasoning
            // as it streams, dimmed and folding away, then its words.
            if let Some(live) = self.live.clone().filter(|_| working) {
                ui.add_space(6.0);
                if !live.reasoning.trim().is_empty() {
                    let title = if live.phase == tequila_ai::Phase::Thinking { "Thinking\u{2026}" } else { "Its reasoning" };
                    let title = egui::RichText::new(title).small().color(theme::dim());
                    egui::CollapsingHeader::new(title).id_salt("assistant-live-reasoning").default_open(true).show(ui, |ui| {
                        egui::ScrollArea::vertical().id_salt("assistant-live-reasoning-text").max_height(150.0).stick_to_bottom(true).show(ui, |ui| {
                            reasoning(ui, &live.reasoning);
                        });
                    });
                }
                if !live.text.trim().is_empty() {
                    reply(ui, live.text.trim());
                }
            }
            if self.setup.is_some() {
                ui.add_space(8.0);
                self.setup_card(ui, around);
            }
            // How far the one answering reaches, where that is further than
            // Tequila's own tools: said before anything is asked of it.
            if let (Some(Backend::Agent(kind)), true) = (self.backend(), items.is_empty()) {
                if let Some(caveat) = kind.caveat() {
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(egui::RichText::new(caveat).small().color(theme::dim())).wrap());
                }
            }
            if let (Some((kind, label)), true) = (&self.account, items.is_empty()) {
                ui.add_space(8.0);
                let paid = match kind.as_str() {
                    "account" => "a subscription",
                    "api_key" => "an API key, by use",
                    "none" => "",
                    _ => "its own account",
                };
                if !paid.is_empty() {
                    let who = if label.is_empty() { String::new() } else { format!(" ({label})") };
                    ui.vertical_centered(|ui| ui.label(egui::RichText::new(format!("Paid for by {paid}{who}.")).small().color(theme::dim())));
                }
            }
            ui.add_space(6.0);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_at_a_glance() {
        assert_eq!(clock(std::time::Duration::from_secs(42)), "0:42");
        assert_eq!(clock(std::time::Duration::from_secs(725)), "12:05");
        assert_eq!(clock(std::time::Duration::from_secs(3723)), "1:02:03");
        assert_eq!(spoken(7.6), "8 s");
        assert_eq!(spoken(125.0), "2 min 5 s");
        assert_eq!(spoken(120.0), "2 min");
    }

    #[test]
    fn a_line_is_split_at_its_bold_and_its_code() {
        assert_eq!(spans("Plain words."), [Span::Plain("Plain words.")]);
        assert_eq!(
            spans("The **grille** is `part 12`, done."),
            [Span::Plain("The "), Span::Bold("grille"), Span::Plain(" is "), Span::Code("part 12"), Span::Plain(", done.")]
        );
        // A mark never closed is words, not formatting.
        assert_eq!(spans("2 ** 3 is eight"), [Span::Plain("2 ** 3 is eight")]);
        assert_eq!(spans("`code` first"), [Span::Code("code"), Span::Plain(" first")]);
        // Stars inside code stay as they are.
        assert_eq!(spans("`a ** b` then **bold**"), [Span::Code("a ** b"), Span::Plain(" then "), Span::Bold("bold")]);
    }

    #[test]
    fn past_chats_are_grouped_by_when() {
        let now = 100_000_000;
        assert_eq!(day(now - 60, now), "Today");
        assert_eq!(day(now - 90_000, now), "Yesterday");
        assert_eq!(day(now - 4 * 86_400, now), "This week");
        assert_eq!(day(now - 30 * 86_400, now), "Earlier");
    }
}
