//! The Layers panel: the part's labels, top of the stack first, each with
//! its artwork, an eye and a lock, its name (double-click to rename), its
//! opacity, and a grip to drag it up or down the stack; then the
//! material's textures on the layout, to select and place.
//!
//! The panel only reports what was asked for (`Action`); the editor makes
//! each one an edit, so a click here and a key on the canvas go the same
//! way.

use super::selection::{Item, Selection};
use super::textures::Layer;
use crate::icons::{self, Icon};
use tequila_materials::decal::Decal;
use tequila_materials::graph::TextureId;
use tequila_texture::projection::Projection;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Select this, or add it to what is selected (Shift).
    Select(Item, bool),
    Visible(usize, bool),
    Locked(usize, bool),
    Rename(usize, String),
    /// A label's opacity; the bool says the drag that set it has ended.
    Opacity(usize, f32, bool),
    /// The label at `from` moved to `to` in the stack, bottom first.
    Reorder { from: usize, to: usize },
    Duplicate(usize),
    Delete(usize),
    /// Add a label from an image file.
    Add,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    renaming: Option<(usize, String, bool)>,
    /// The label whose grip is being dragged.
    dragging: Option<usize>,
}

const ROW: f32 = 34.0;
const THUMB: f32 = 26.0;

/// A small square of the artwork, letterboxed, on a well.
fn thumbnail(ui: &egui::Ui, rect: egui::Rect, texture: Option<&egui::TextureHandle>) {
    let p = crate::theme::p();
    ui.painter().rect_filled(rect, 4.0, p.field);
    match texture {
        Some(texture) => {
            let size = texture.size_vec2();
            let k = (rect.width() / size.x.max(1.0)).min(rect.height() / size.y.max(1.0));
            let shown = egui::Rect::from_center_size(rect.center(), size * k);
            ui.painter().image(texture.id(), shown, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        None => icons::paint(ui.painter(), rect.shrink(6.0), Icon::Texture, p.text_faint),
    }
}

/// The panel. `thumbnail_of` reads an artwork's picture once; `can_add`
/// says whether there is a material to add a label to.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut State,
    labels: &[Decal],
    textures: &[Layer],
    selection: &Selection,
    thumbnail_of: &mut dyn FnMut(TextureId) -> Option<egui::TextureHandle>,
    can_add: bool,
) -> Vec<Action> {
    let p = crate::theme::p();
    let mut out = Vec::new();
    let shift = ui.input(|i| i.modifiers.shift);
    if labels.is_empty() {
        ui.weak(if can_add { "No labels yet. Add artwork to print on the part." } else { "This part has no material to put labels on." });
    }
    let count = labels.len();
    let mut rows: Vec<(usize, egui::Rect)> = Vec::with_capacity(count);
    for index in (0..count).rev() {
        let label = &labels[index];
        let selected = selection.contains(Item::Label(index));
        let (rect, row) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click());
        rows.push((index, rect));
        let hovered = row.hovered() || state.dragging == Some(index);
        if selected {
            ui.painter().rect_filled(rect, 6.0, p.accent_fill);
        } else if hovered {
            ui.painter().rect_filled(rect, 6.0, p.hover.gamma_multiply(0.5));
        }
        let mut x = rect.left() + 2.0;
        let middle = rect.center().y;
        // The grip, to drag the label up or down the stack.
        let grip = egui::Rect::from_min_size(egui::pos2(x, middle - 10.0), egui::vec2(14.0, 20.0));
        let grip_response = ui.interact(grip, ui.id().with(("layer-grip", index)), egui::Sense::drag());
        icons::paint(ui.painter(), grip.shrink(2.0), Icon::Grip, if grip_response.hovered() || state.dragging == Some(index) { p.text } else { p.text_faint });
        if grip_response.drag_started() {
            state.dragging = Some(index);
        }
        let grip_response = grip_response.on_hover_text("Drag up or down the stack");
        if grip_response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        x += 16.0;
        let thumb = egui::Rect::from_min_size(egui::pos2(x, middle - THUMB * 0.5), egui::Vec2::splat(THUMB));
        thumbnail(ui, thumb, thumbnail_of(label.texture).as_ref());
        x += THUMB + 6.0;

        // From the right: delete, duplicate, lock, eye, opacity.
        let mut right = rect.right() - 2.0;
        let mut button = |ui: &mut egui::Ui, icon: Icon, on: bool, id: &str, title: &str, detail: &str| {
            right -= 24.0;
            let at = egui::Rect::from_min_size(egui::pos2(right, middle - 12.0), egui::Vec2::splat(24.0));
            let response = ui.interact(at, ui.id().with((id, index)), egui::Sense::click());
            icons::paint_button(ui, at, &response, icon, if on { icons::Tone::On } else { icons::Tone::Quiet }, true);
            icons::tip(response, title, detail, "")
        };
        if button(ui, Icon::Trash, false, "layer-delete", "Delete", "Remove this label").clicked() {
            out.push(Action::Delete(index));
        }
        if button(ui, Icon::Copy, false, "layer-duplicate", "Duplicate", "A copy of this label, on top of it").clicked() {
            out.push(Action::Duplicate(index));
        }
        if button(ui, if label.locked { Icon::Lock } else { Icon::Unlock }, label.locked, "layer-lock", if label.locked { "Unlock" } else { "Lock" }, "A locked label cannot be picked or moved on the canvas")
            .clicked()
        {
            out.push(Action::Locked(index, !label.locked));
        }
        if button(ui, if label.enabled { Icon::Eye } else { Icon::EyeClosed }, false, "layer-eye", if label.enabled { "Hide" } else { "Show" }, "A hidden label does not print, and cannot be picked on the canvas")
            .clicked()
        {
            out.push(Action::Visible(index, !label.enabled));
        }
        right -= 62.0;
        let opacity_at = egui::Rect::from_min_size(egui::pos2(right, middle - 12.0), egui::vec2(58.0, 24.0));
        let mut percent = label.opacity * 100.0;
        let slider = ui
            .put(opacity_at, crate::field::Slider::new(&mut percent, 0.0..=100.0).suffix("%").fixed_decimals(0))
            .on_hover_text("Opacity: how strongly it prints");
        if slider.changed() {
            out.push(Action::Opacity(index, (percent / 100.0).clamp(0.0, 1.0), false));
        }
        if slider.drag_stopped() || slider.lost_focus() {
            out.push(Action::Opacity(index, (percent / 100.0).clamp(0.0, 1.0), true));
        }

        // The name, or the field it is renamed in.
        let name_at = egui::Rect::from_min_max(egui::pos2(x, rect.top() + 3.0), egui::pos2(right - 4.0, rect.bottom() - 3.0));
        match &mut state.renaming {
            Some((at, text, fresh)) if *at == index => {
                let edit = ui.put(name_at, egui::TextEdit::singleline(text).desired_width(name_at.width()));
                if *fresh {
                    edit.request_focus();
                    *fresh = false;
                }
                let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if edit.lost_focus() || escape {
                    if !escape && !text.trim().is_empty() && text.trim() != label.name {
                        out.push(Action::Rename(index, text.trim().to_string()));
                    }
                    state.renaming = None;
                }
            }
            _ => {
                let colour = if label.enabled { p.text } else { p.text_faint };
                let name = egui::RichText::new(&label.name).color(colour);
                let note = if label.mapping.projection != Projection::Uv { "projected" } else { "" };
                let galley = ui.painter().layout(name.text().to_string(), egui::FontId::proportional(crate::theme::BODY - 0.5), colour, name_at.width());
                let lines = galley.rows.len();
                ui.painter().galley(egui::pos2(name_at.left(), if note.is_empty() || lines > 1 { middle - galley.size().y * 0.5 } else { middle - galley.size().y + 1.0 }), galley, colour);
                if !note.is_empty() && lines == 1 {
                    ui.painter().text(egui::pos2(name_at.left(), middle + 1.0), egui::Align2::LEFT_TOP, note, egui::FontId::proportional(10.5), p.text_faint);
                }
            }
        }

        let row = row.on_hover_text(if label.locked { "Locked: unlock it to select it" } else { "Click to select, Shift-click to add. Double-click to rename." });
        if row.double_clicked() {
            state.renaming = Some((index, label.name.clone(), true));
        } else if row.clicked() && !label.locked {
            out.push(Action::Select(Item::Label(index), shift));
        }
    }

    // A grip let go over another row: the label goes to that row's place.
    if let Some(from) = state.dragging {
        let pointer = ui.ctx().pointer_interact_pos().or_else(|| ui.ctx().pointer_hover_pos());
        let over = pointer.and_then(|at| {
            rows.iter().min_by(|a, b| (a.1.center().y - at.y).abs().total_cmp(&(b.1.center().y - at.y).abs())).map(|(index, rect)| (*index, *rect))
        });
        if let Some((to, rect)) = over {
            if to != from {
                // A line where it would land: above the row when it goes up
                // the stack, below when it goes down.
                let y = if to > from { rect.top() } else { rect.bottom() };
                ui.painter().line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], egui::Stroke::new(2.0_f32, p.accent));
            }
        }
        if ui.input(|i| i.pointer.any_released()) {
            if let Some((to, _)) = over {
                if to != from {
                    out.push(Action::Reorder { from, to });
                }
            }
            state.dragging = None;
        }
    }

    ui.add_space(4.0);
    ui.add_enabled_ui(can_add, |ui| {
        let add = ui.button("+ Add label…").on_hover_text("Print artwork from an image file on the part, placed on its UV layout").on_disabled_hover_text("This part has no material to put labels on");
        if add.clicked() {
            out.push(Action::Add);
        }
    });

    if !textures.is_empty() {
        ui.add_space(6.0);
        crate::theme::section(ui, "Textures on the layout");
        for layer in textures {
            let item = Item::Texture(layer.param);
            let selected = selection.contains(item);
            let (rect, row) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click());
            if selected {
                ui.painter().rect_filled(rect, 6.0, p.accent_fill);
            } else if row.hovered() {
                ui.painter().rect_filled(rect, 6.0, p.hover.gamma_multiply(0.5));
            }
            let thumb = egui::Rect::from_min_size(egui::pos2(rect.left() + 18.0, rect.center().y - THUMB * 0.5), egui::Vec2::splat(THUMB));
            thumbnail(ui, thumb, layer.texture.and_then(&mut *thumbnail_of).as_ref());
            let text_at = egui::pos2(thumb.right() + 6.0, rect.center().y);
            ui.painter().text(text_at - egui::vec2(0.0, 1.0), egui::Align2::LEFT_BOTTOM, layer.param.label(), egui::FontId::proportional(crate::theme::BODY - 0.5), p.text);
            ui.painter().text(text_at + egui::vec2(0.0, 1.0), egui::Align2::LEFT_TOP, &layer.name, egui::FontId::proportional(10.5), p.text_faint);
            if row.on_hover_text("Select to place it: drag its tile, its corner to change how often it repeats, its stalk to turn it").clicked() {
                out.push(Action::Select(item, false));
            }
        }
    }
    out
}
