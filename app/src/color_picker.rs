//! Colour pickers for colours in the rendering space.
//!
//! Every colour a material, light or environment holds is in the rendering
//! space, ACEScg. egui's picker is linear sRGB. Handing it the ACEScg numbers
//! as they stand showed a swatch of a different colour from the one that
//! renders — a saturated red rendered brighter and redder than the swatch it
//! was picked from — so every picker goes through here.

use eframe::egui;
use tequila_core::spectrum::Spectrum;

/// What the picker shows for a colour: linear sRGB, clipped to what a
/// swatch can show.
pub fn to_picker(color: Spectrum) -> [f32; 3] {
    let rgb = color.to_linear_srgb();
    [rgb.x.max(0.0), rgb.y.max(0.0), rgb.z.max(0.0)]
}

/// The colour a picked value stands for.
pub fn from_picker(rgb: [f32; 3]) -> Spectrum {
    Spectrum::from_linear_srgb(tequila_core::math::vec3(rgb[0], rgb[1], rgb[2]))
}

/// A picker button for `color`, writing back only when the user picks: a
/// rounded swatch, ringed so a colour close to the panel's still shows
/// where it ends.
pub fn edit(ui: &mut egui::Ui, color: &mut Spectrum) -> egui::Response {
    let mut rgb = to_picker(*color);
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(40.0, 22.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = crate::theme::p();
        let ring = if response.hovered() || is_open(ui, &response) { p.text_dim } else { p.line_strong };
        ui.painter().rect(rect, 6.0, swatch(rgb), egui::Stroke::new(1.0_f32, ring));
    }
    picker(ui, &mut response, &mut rgb);
    if response.changed() {
        *color = from_picker(rgb);
    }
    response
}

/// A colour as a field the width of the column: its name at the left, as a
/// number field's is, and its swatch and hex at the right — so a colour
/// lines up with the fields around it instead of starting a row of its
/// own.
pub fn field(ui: &mut egui::Ui, label: impl Into<egui::WidgetText>, color: &mut Spectrum) -> egui::Response {
    let mut rgb = to_picker(*color);
    let width = ui.available_width().max(80.0);
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(width, 26.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = crate::theme::p();
        let lit = response.hovered() || is_open(ui, &response);
        let ink = |c: egui::Color32| if ui.is_enabled() { c } else { c.gamma_multiply(0.45) };
        let painter = ui.painter();
        painter.rect_filled(rect, 5.0, crate::field::well(&p, lit));
        let chip = egui::Rect::from_min_size(egui::pos2(rect.right() - 4.0 - 34.0, rect.top() + 4.0), egui::vec2(34.0, rect.height() - 8.0));
        painter.rect(chip, 4.0, swatch(rgb), egui::Stroke::new(1.0_f32, p.line_strong));
        let c = swatch(rgb);
        let hex = painter.layout_no_wrap(
            format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b()),
            egui::FontId::proportional(crate::theme::BODY - 1.5),
            ink(if lit { p.text } else { p.text_dim }),
        );
        let hex_at = egui::pos2(chip.left() - 8.0 - hex.size().x, rect.center().y - hex.size().y * 0.5);
        let room = (hex_at.x - 8.0 - (rect.left() + 9.0)).max(0.0);
        let name = label.into().into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            room,
            egui::FontId::proportional(crate::theme::BODY - 0.5),
        );
        painter.galley(egui::pos2(rect.left() + 9.0, rect.center().y - name.size().y * 0.5), name, ink(p.text_dim));
        painter.galley(hex_at, hex, egui::Color32::PLACEHOLDER);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    picker(ui, &mut response, &mut rgb);
    if response.changed() {
        *color = from_picker(rgb);
    }
    response
}

/// How a linear sRGB colour shows on screen.
fn swatch(rgb: [f32; 3]) -> egui::Color32 {
    egui::Color32::from(egui::Rgba::from_rgb(rgb[0], rgb[1], rgb[2]))
}

fn popup_id(response: &egui::Response) -> egui::Id {
    response.id.with("picker")
}

fn is_open(ui: &egui::Ui, response: &egui::Response) -> bool {
    ui.memory(|m| m.is_popup_open(popup_id(response)))
}

/// egui's picker, opened by a click on `response` and shown below it,
/// editing `rgb`; marks `response` changed when the colour moves.
///
/// The hue is kept while the picker is open: a colour dragged through grey
/// or black has no hue of its own, and reading it back from the numbers
/// each frame would snap the picker's hue to red.
fn picker(ui: &mut egui::Ui, response: &mut egui::Response, rgb: &mut [f32; 3]) {
    use egui::widgets::color_picker::{color_picker_hsva_2d, Alpha};
    let id = popup_id(response);
    if response.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    if !ui.memory(|m| m.is_popup_open(id)) {
        return;
    }
    let kept: Option<egui::ecolor::Hsva> = ui.data(|d| d.get_temp(id));
    let mut hsva = match kept {
        Some(h) if h.to_rgb().iter().zip(rgb.iter()).all(|(a, b)| (a - b).abs() < 1e-4) => h,
        _ => egui::ecolor::Hsva::from_rgb(*rgb),
    };
    let at = response.rect.left_bottom() + egui::vec2(0.0, 4.0);
    let area = egui::Area::new(id)
        .kind(egui::UiKind::Picker)
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .constrain(true)
        .show(ui.ctx(), |ui| {
            ui.spacing_mut().slider_width = 240.0;
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                if color_picker_hsva_2d(ui, &mut hsva, Alpha::Opaque) {
                    *rgb = hsva.to_rgb();
                    response.mark_changed();
                }
            });
        })
        .response;
    ui.data_mut(|d| d.insert_temp(id, hsva));
    if !response.clicked() && (ui.input(|i| i.key_pressed(egui::Key::Escape)) || area.clicked_elsewhere()) {
        ui.memory_mut(|m| m.close_popup());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A colour picked renders as the colour picked, and shows as itself
    /// when the picker opens again.
    #[test]
    fn a_picked_colour_renders_as_the_colour_picked() {
        for rgb in [[0.8f32, 0.03, 0.04], [0.05, 0.3, 0.9], [0.9, 0.75, 0.05], [0.5, 0.5, 0.5]] {
            let stored = from_picker(rgb);
            let rendered = stored.to_linear_srgb();
            let shown = to_picker(stored);
            for c in 0..3 {
                assert!(
                    (rendered[c] - rgb[c]).abs() < 1e-3,
                    "picked {rgb:?}, which renders as {rendered:?}"
                );
                assert!((shown[c] - rgb[c]).abs() < 1e-3, "picked {rgb:?}, shown again as {shown:?}");
            }
        }
    }
}
