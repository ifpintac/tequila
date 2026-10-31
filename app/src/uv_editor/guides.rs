//! Guides in the editor: a guide's place typed exactly, after a
//! double-click on it. Dragging them out of the rulers, moving them and
//! dropping them back is the canvas's (`super::canvas`); what they are and
//! how they are saved, `tequila_scene::guides`.

use super::units::Scale;
use tequila_scene::guides::{GuideAxis, UvGuide};

/// A guide's place being typed.
#[derive(Clone, Debug)]
pub struct Typing {
    pub index: usize,
    pub axis: GuideAxis,
    pub text: String,
    /// Where the popup opens, on the screen.
    pub at: egui::Pos2,
    fresh: bool,
}

impl Typing {
    pub fn new(index: usize, guide: UvGuide, scale: Scale, at: egui::Pos2) -> Typing {
        Typing { index, axis: guide.axis, text: format!("{:.*}", scale.decimals(), scale.shown(guide.at)), at, fresh: true }
    }
}

/// What came of the typing: the new place, in layout units, once Enter is
/// pressed; nothing, once it is given up.
pub enum Typed {
    Open,
    Place(f32),
    Cancelled,
}

/// The popup by the guide: a field for its place, in the editor's unit.
pub fn show(ctx: &egui::Context, typing: &mut Typing, scale: Scale) -> Typed {
    let mut out = Typed::Open;
    let p = crate::theme::p();
    egui::Area::new(egui::Id::new("uv-guide-typing"))
        .order(egui::Order::Foreground)
        .fixed_pos(typing.at + egui::vec2(10.0, 10.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(p.raised).show(ui, |ui| {
                ui.label(egui::RichText::new(match typing.axis {
                    GuideAxis::U => "Guide down the layout, across at",
                    GuideAxis::V => "Guide across the layout, down at",
                }).color(p.text_dim).small());
                ui.horizontal(|ui| {
                    let edit = ui.add(egui::TextEdit::singleline(&mut typing.text).desired_width(90.0));
                    ui.label(scale.unit.suffix());
                    if typing.fresh {
                        edit.request_focus();
                        typing.fresh = false;
                    }
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if escape {
                        out = Typed::Cancelled;
                    } else if enter || edit.lost_focus() {
                        out = match crate::numeric::parse(&typing.text, scale.unit.numeric()) {
                            Some(value) if value.is_finite() => {
                                // A length typed with a unit is parsed into
                                // the field's own unit.
                                let place = scale.layout(value as f32);
                                if UvGuide::new(typing.axis, place).is_valid() {
                                    Typed::Place(place)
                                } else {
                                    Typed::Cancelled
                                }
                            }
                            _ => Typed::Cancelled,
                        };
                    }
                });
            });
        });
    out
}
