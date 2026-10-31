//! The Transform panel: where the selection is and how large, typed.
//!
//! X and Y are the place of one of nine points of its frame - a corner, the
//! middle of an edge or the middle, picked as in Illustrator - and width and
//! height its size, in the part's millimetres where the layout has one size
//! and in layout units where it has not. One label is described in its own
//! frame, turned as it is; several by the box square to the layout round
//! them, which a typed turn turns as a group.

use super::placement;
use super::tools::Frame;
use super::units::Scale;
use tequila_core::math::{vec2, Vec2};
use tequila_texture::projection::Mapping;

/// One of the nine points of a frame: -1, 0 or 1 across and down from its
/// middle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Reference {
    pub across: i8,
    pub down: i8,
}

impl Reference {
    pub fn point(self, frame: &Frame) -> Vec2 {
        frame.handle(self.across as f32, self.down as f32)
    }

    fn name(self) -> &'static str {
        match (self.across, self.down) {
            (-1, -1) => "Top left",
            (0, -1) => "Top middle",
            (1, -1) => "Top right",
            (-1, 0) => "Left middle",
            (0, 0) => "Centre",
            (1, 0) => "Right middle",
            (-1, 1) => "Bottom left",
            (0, 1) => "Bottom middle",
            _ => "Bottom right",
        }
    }
}

/// The frame moved so `reference` is at `at`.
pub fn with_point(frame: Frame, reference: Reference, at: Vec2) -> Frame {
    Frame { centre: frame.centre + (at - reference.point(&frame)), ..frame }
}

/// The frame sized to `size`, `reference` staying where it was.
pub fn resized(frame: Frame, reference: Reference, size: Vec2) -> Frame {
    let fixed = reference.point(&frame);
    let sized = Frame { size: size.max(Vec2::splat(placement::SMALLEST)), ..frame };
    with_point(sized, reference, fixed)
}

/// The frame turned to `angle`, about `reference`.
pub fn turned(frame: Frame, reference: Reference, angle: f32) -> Frame {
    let fixed = reference.point(&frame);
    with_point(Frame { angle, ..frame }, reference, fixed)
}

/// A label's own frame.
pub fn frame_of(mapping: &Mapping) -> Frame {
    Frame { centre: placement::centre(mapping), size: placement::size(mapping), angle: placement::turn(mapping) }
}

/// The box square to the layout round several labels.
pub fn frame_round(mappings: &[Mapping]) -> Option<Frame> {
    let boxes: Vec<_> = mappings.iter().map(placement::bounds).collect();
    let (low, high) = super::align::union(&boxes)?;
    Some(Frame { centre: (low + high) * 0.5, size: high - low, angle: 0.0 })
}

/// One label given the frame `to`.
pub fn apply_one(mapping: Mapping, to: Frame) -> Mapping {
    placement::centred(placement::turned(placement::resized(mapping, to.size), to.angle), to.centre)
}

/// One of several labels carried by their frame's change from `from` to
/// `to`: scaled and turned about `reference`, then moved with it.
pub fn apply_group(mapping: Mapping, from: Frame, to: Frame, reference: Reference) -> Mapping {
    let pivot = reference.point(&from);
    let k = vec2(to.size.x / from.size.x.max(1e-9), to.size.y / from.size.y.max(1e-9));
    let mut out = mapping;
    if (k - Vec2::ONE).length() > 1e-6 {
        out = placement::scaled_about(out, pivot, k);
    }
    let turn = to.angle - from.angle;
    if turn.abs() > 1e-7 {
        out = placement::turned_about(out, pivot, turn);
    }
    let shift = reference.point(&to) - pivot;
    if shift.length() > 0.0 {
        out = placement::moved(out, shift);
    }
    out
}

/// The panel's own state: the point its numbers refer to, and whether width
/// and height keep in proportion; and for the layout's own, a point other
/// than that one its turns and scales go about - the 2D cursor - set each
/// frame.
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    pub reference: Reference,
    pub keep_aspect: bool,
    pub about: Option<Vec2>,
}

impl Default for Panel {
    fn default() -> Self {
        Panel { reference: Reference::default(), keep_aspect: true, about: None }
    }
}

/// The nine-point picker: a box of three by three, the chosen point filled.
fn picker(ui: &mut egui::Ui, reference: &mut Reference) {
    let p = crate::theme::p();
    let side = 46.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    let inner = rect.shrink(7.0);
    ui.painter().rect_stroke(inner, 1.0, egui::Stroke::new(1.0_f32, p.line_strong));
    for down in -1i8..=1 {
        for across in -1i8..=1 {
            let at = egui::pos2(inner.center().x + across as f32 * inner.width() * 0.5, inner.center().y + down as f32 * inner.height() * 0.5);
            let hit = egui::Rect::from_center_size(at, egui::Vec2::splat(13.0));
            let id = ui.id().with(("reference", across, down));
            let response = ui.interact(hit, id, egui::Sense::click());
            let chosen = *reference == Reference { across, down };
            let colour = if chosen { p.accent } else if response.hovered() { p.text } else { p.text_faint };
            if chosen {
                ui.painter().rect_filled(egui::Rect::from_center_size(at, egui::Vec2::splat(7.0)), 1.0, colour);
            } else {
                ui.painter().rect_stroke(egui::Rect::from_center_size(at, egui::Vec2::splat(6.0)), 1.0, egui::Stroke::new(1.2_f32, colour));
            }
            let target = Reference { across, down };
            if response.on_hover_text(format!("{}: the point X and Y give the place of", target.name())).clicked() {
                *reference = target;
            }
        }
    }
}

/// A length field in the editor's unit; true when it was changed.
fn length(ui: &mut egui::Ui, prefix: &str, value: &mut f32, scale: Scale, tip: &str, positive: bool) -> bool {
    let mut shown = scale.shown(*value);
    let speed = match scale.unit {
        super::units::Unit::Millimetres => 0.1,
        super::units::Unit::Centimetres => 0.01,
        super::units::Unit::Inches => 0.005,
        super::units::Unit::Layout => 0.001,
    };
    let unit = scale.unit.numeric();
    let low = if positive { scale.shown(placement::SMALLEST) } else { f32::MIN };
    // A value already outside the range - an edge's box, as good as no
    // width - is shown as it is: clamped, the field would report a change
    // nobody made, and the selection would be scaled by it.
    let field = egui::DragValue::new(&mut shown)
        .speed(speed)
        .range(low..=f32::MAX)
        .clamp_existing_to_range(false)
        .prefix(format!("{prefix} "))
        .suffix(format!(" {}", scale.unit.suffix()))
        .max_decimals(scale.decimals() + 1)
        .custom_parser(move |text| {
            // The layout's own unit is not a length to convert from.
            let bare = text.trim().trim_start_matches(prefix_of(text)).trim().trim_end_matches("UV").trim();
            crate::numeric::parse(bare, unit)
        });
    let response = ui.add_sized([96.0, 24.0], field).on_hover_text(tip);
    if response.changed() && shown.is_finite() {
        *value = scale.layout(shown);
        return true;
    }
    false
}

/// A typed field's leading name, `X ` or `W `, which a person may leave in.
fn prefix_of(text: &str) -> &str {
    let t = text.trim_start();
    match t.split_once(' ') {
        Some((head, _)) if head.chars().all(|c| c.is_ascii_alphabetic()) && head.len() <= 2 => head,
        _ => "",
    }
}

/// The panel for labels: `selected` places in `labels`, which it changes in
/// place. True when it did.
pub fn labels(ui: &mut egui::Ui, panel: &mut Panel, labels: &mut [tequila_materials::decal::Decal], selected: &[usize], scale: Scale) -> bool {
    let mappings: Vec<Mapping> = selected.iter().filter_map(|&i| labels.get(i)).map(|d| d.mapping).collect();
    let one = mappings.len() == 1;
    let Some(frame) = (if one { Some(frame_of(&mappings[0])) } else { frame_round(&mappings) }) else {
        ui.weak("Select a label to place it by number.");
        return false;
    };
    let mut to = frame;
    let mut changed = false;
    let mut flip: Option<bool> = None;
    ui.horizontal(|ui| {
        picker(ui, &mut panel.reference);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            let mut at = panel.reference.point(&frame);
            ui.horizontal(|ui| {
                let x = length(ui, "X", &mut at.x, scale, "Across the layout, from its left edge, to the chosen point", false);
                let y = length(ui, "Y", &mut at.y, scale, "Down the layout, from its top edge, to the chosen point", false);
                if x || y {
                    to = with_point(frame, panel.reference, at);
                    changed = true;
                }
            });
            ui.horizontal(|ui| {
                let mut size = frame.size;
                let w = length(ui, "W", &mut size.x, scale, if one { "The label's width, as it is turned" } else { "The width of the box round the selection" }, true);
                let h = length(ui, "H", &mut size.y, scale, if one { "The label's height" } else { "The height of the box round the selection" }, true);
                if panel.keep_aspect {
                    if w {
                        size.y = frame.size.y * size.x / frame.size.x.max(1e-9);
                    } else if h {
                        size.x = frame.size.x * size.y / frame.size.y.max(1e-9);
                    }
                }
                let lock = crate::icons::button(ui, if panel.keep_aspect { crate::icons::Icon::Link } else { crate::icons::Icon::Unlink }, panel.keep_aspect, 24.0, egui::Sense::click());
                if crate::icons::tip(lock, "Keep proportions", "Width and height change together", "").clicked() {
                    panel.keep_aspect = !panel.keep_aspect;
                }
                if w || h {
                    to = resized(frame, panel.reference, size);
                    changed = true;
                }
            });
        });
    });
    ui.horizontal(|ui| {
        // Counted anticlockwise, as the Labels tab's rotation is.
        let mut degrees = if one { -frame.angle.to_degrees() } else { 0.0 };
        // Never "-0.0": a label not turned reads 0.
        if degrees.abs() < 5e-5 {
            degrees = 0.0;
        }
        let response = ui
            .add_sized([96.0, 24.0], egui::DragValue::new(&mut degrees).speed(0.5).prefix(if one { "Turn " } else { "Turn by " }).suffix("°").max_decimals(2))
            .on_hover_text(if one { "How far the label is turned, anticlockwise" } else { "Turn the selection as a group, about the chosen point" });
        if response.changed() && degrees.is_finite() {
            // One label's turn is set; several are turned by what is typed.
            let angle = -degrees.to_radians();
            to = turned(frame, panel.reference, if one { angle } else { frame.angle + angle });
            changed = true;
        }
        let across = crate::icons::button(ui, crate::icons::Icon::FlipAcross, false, 24.0, egui::Sense::click());
        if crate::icons::tip(across, "Flip horizontal", "Mirror left to right, where it sits", "").clicked() {
            flip = Some(true);
        }
        let down = crate::icons::button(ui, crate::icons::Icon::FlipDown, false, 24.0, egui::Sense::click());
        if crate::icons::tip(down, "Flip vertical", "Mirror top to bottom, where it sits", "").clicked() {
            flip = Some(false);
        }
    });
    // Opacity, shared where the selection's labels share it.
    let mut opacity = selected.iter().filter_map(|&i| labels.get(i)).map(|d| d.opacity).fold(f32::NAN, |a, b| if a.is_nan() { b } else { a.max(b) });
    if opacity.is_nan() {
        opacity = 1.0;
    }
    let mut percent = opacity * 100.0;
    let opacity_changed = ui
        .add(crate::field::Slider::new(&mut percent, 0.0..=100.0).text("Opacity").suffix("%").fixed_decimals(0))
        .on_hover_text("How strongly the artwork prints over the material")
        .changed();

    if changed {
        for &i in selected {
            if let Some(label) = labels.get_mut(i) {
                label.mapping = if one { apply_one(label.mapping, to) } else { apply_group(label.mapping, frame, to, panel.reference) };
            }
        }
    }
    if let Some(across) = flip {
        let middle = frame.centre;
        for &i in selected {
            if let Some(label) = labels.get_mut(i) {
                label.mapping = if one { placement::flipped(label.mapping, across) } else { placement::flipped_about(label.mapping, middle, across) };
            }
        }
        changed = true;
    }
    if opacity_changed {
        for &i in selected {
            if let Some(label) = labels.get_mut(i) {
                label.opacity = (percent / 100.0).clamp(0.0, 1.0);
            }
        }
        changed = true;
    }
    changed
}

/// What the panel did to a selection of the layout's own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayoutChange {
    /// Moved, sized or turned: the map from where it is shown to where the
    /// numbers put it.
    Map(tequila_scene::uv::UvAffine),
    /// Mirrored left to right (true) or top to bottom.
    Flip(bool),
    /// Turned a quarter, clockwise as shown, or back.
    Quarter(i32),
}

/// The panel for islands, edges or vertices: the box round them, its place
/// and size, a turn by so many degrees, flips and quarter turns. The change,
/// if there was one.
pub fn layout(ui: &mut egui::Ui, panel: &mut Panel, frame: &Frame, scale: Scale) -> Option<LayoutChange> {
    let mut out = None;
    let point = panel.reference.point(frame);
    let pivot = panel.about.unwrap_or(point);
    ui.horizontal(|ui| {
        picker(ui, &mut panel.reference);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            let mut at = point;
            ui.horizontal(|ui| {
                let x = length(ui, "X", &mut at.x, scale, "Across the layout, from its left edge, to the chosen point", false);
                let y = length(ui, "Y", &mut at.y, scale, "Down the layout, from its top edge, to the chosen point", false);
                if x || y {
                    out = Some(LayoutChange::Map(super::editing::moved(at - point)));
                }
            });
            ui.horizontal(|ui| {
                let mut size = frame.size;
                let w = length(ui, "W", &mut size.x, scale, "The width of the box round the selection", true);
                let h = length(ui, "H", &mut size.y, scale, "The height of the box round the selection", true);
                if panel.keep_aspect {
                    if w {
                        size.y = frame.size.y * size.x / frame.size.x.max(1e-9);
                    } else if h {
                        size.x = frame.size.x * size.y / frame.size.y.max(1e-9);
                    }
                }
                let lock = crate::icons::button(ui, if panel.keep_aspect { crate::icons::Icon::Link } else { crate::icons::Icon::Unlink }, panel.keep_aspect, 24.0, egui::Sense::click());
                if crate::icons::tip(lock, "Keep proportions", "Width and height change together", "").clicked() {
                    panel.keep_aspect = !panel.keep_aspect;
                }
                // A box with no width, or no height - the edges along a
                // line - has nothing to scale that way.
                let flat = |extent: f32| extent <= 1e-6;
                if w && flat(frame.size.x) || h && flat(frame.size.y) {
                    size = frame.size;
                }
                if size != frame.size && !flat(frame.size.x) && !flat(frame.size.y) {
                    let to = resized(*frame, panel.reference, size);
                    out = Some(LayoutChange::Map(super::editing::panel_change(frame, &to, pivot, pivot)));
                } else if size != frame.size {
                    // One way only, the other kept as it is.
                    let k = vec2(if flat(frame.size.x) { 1.0 } else { size.x / frame.size.x }, if flat(frame.size.y) { 1.0 } else { size.y / frame.size.y });
                    out = Some(LayoutChange::Map(super::editing::scaled(pivot, k)));
                }
            });
        });
    });
    ui.horizontal(|ui| {
        let mut degrees = 0.0f32;
        let response = ui
            .add_sized([96.0, 24.0], egui::DragValue::new(&mut degrees).speed(0.5).prefix("Turn by ").suffix("°").max_decimals(2))
            .on_hover_text("Turn the selection, anticlockwise, about the chosen point");
        if response.changed() && degrees.is_finite() && degrees != 0.0 {
            out = Some(LayoutChange::Map(super::editing::turned(pivot, -degrees.to_radians())));
        }
        for (icon, title, detail, change) in [
            (crate::icons::Icon::QuarterCcw, "Turn a quarter anticlockwise", "About the selection's middle", LayoutChange::Quarter(-1)),
            (crate::icons::Icon::QuarterCw, "Turn a quarter clockwise", "About the selection's middle", LayoutChange::Quarter(1)),
            (crate::icons::Icon::FlipAcross, "Flip horizontal", "Mirror left to right about the selection's middle. A mirrored island prints its artwork backwards", LayoutChange::Flip(true)),
            (crate::icons::Icon::FlipDown, "Flip vertical", "Mirror top to bottom about the selection's middle. A mirrored island prints its artwork backwards", LayoutChange::Flip(false)),
        ] {
            let keys = match change {
                LayoutChange::Quarter(1) => "]",
                LayoutChange::Quarter(_) => "[",
                LayoutChange::Flip(true) => "Shift + H",
                _ => "Shift + V",
            };
            let response = crate::icons::button(ui, icon, false, 24.0, egui::Sense::click());
            if crate::icons::tip(response, title, detail, keys).clicked() {
                out = Some(change);
            }
        }
    });
    out
}

/// The panel for a texture: its offset, repeats and turn. The changed
/// mapping, if it was.
pub fn texture(ui: &mut egui::Ui, mapping: &Mapping) -> Option<Mapping> {
    let mut out = *mapping;
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Offset").color(crate::theme::dim()));
        changed |= ui.add(egui::DragValue::new(&mut out.offset.x).speed(0.005).prefix("U ").max_decimals(3)).on_hover_text("Slides the texture across the layout, in tiles").changed();
        changed |= ui.add(egui::DragValue::new(&mut out.offset.y).speed(0.005).prefix("V ").max_decimals(3)).on_hover_text("Slides the texture down the layout, in tiles").changed();
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Repeats").color(crate::theme::dim()));
        let (mut u, mut v) = out.repeats();
        let a = ui.add(egui::DragValue::new(&mut u).speed(0.02).range(0.01..=10_000.0).prefix("U ").max_decimals(2)).on_hover_text("How many times the texture repeats across the layout's square").changed();
        let b = ui.add(egui::DragValue::new(&mut v).speed(0.02).range(0.01..=10_000.0).prefix("V ").max_decimals(2)).on_hover_text("How many times it repeats down").changed();
        if a || b {
            out = placement::tile_resized(out, vec2(1.0 / u.max(0.01), 1.0 / v.max(0.01)));
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        let mut degrees = -placement::turn(&out).to_degrees();
        if degrees.abs() < 5e-5 {
            degrees = 0.0;
        }
        if ui
            .add_sized([96.0, 24.0], egui::DragValue::new(&mut degrees).speed(0.5).prefix("Turn ").suffix("°").max_decimals(2))
            .on_hover_text("How far the texture is turned, anticlockwise, about a tile's middle")
            .changed()
        {
            out = placement::tile_turned(out, -degrees.to_radians() - placement::turn(&out));
            changed = true;
        }
    });
    changed.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_texture::projection::Projection;

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-4
    }

    fn label(at: Vec2, repeats: Vec2) -> Mapping {
        let mapping = Mapping { projection: Projection::Uv, ..Default::default() };
        placement::centred(placement::with_repeats(mapping, repeats), at)
    }

    /// The nine points of a frame, and the frame moved, sized and turned
    /// holding the one chosen.
    #[test]
    fn a_frame_moves_sizes_and_turns_about_its_chosen_point() {
        let frame = Frame { centre: vec2(0.5, 0.5), size: vec2(0.4, 0.2), angle: 0.0 };
        let top_left = Reference { across: -1, down: -1 };
        assert!(near(top_left.point(&frame), vec2(0.3, 0.4)));
        let moved = with_point(frame, top_left, vec2(0.0, 0.0));
        assert!(near(moved.centre, vec2(0.2, 0.1)) && moved.size == frame.size);
        let wider = resized(frame, top_left, vec2(0.8, 0.2));
        assert!(near(top_left.point(&wider), vec2(0.3, 0.4)), "the corner stays");
        assert!(near(wider.centre, vec2(0.7, 0.5)));
        let bottom = Reference { across: 0, down: 1 };
        let quarter = turned(frame, bottom, std::f32::consts::FRAC_PI_2);
        assert!(near(bottom.point(&quarter), bottom.point(&frame)), "the bottom middle stays");
    }

    /// Typed numbers for one label land it where they say: its top left
    /// corner at a place, its width doubled about that corner.
    #[test]
    fn one_label_takes_its_numbers() {
        let mapping = label(vec2(0.5, 0.5), vec2(4.0, 8.0));
        let frame = frame_of(&mapping);
        assert!(near(frame.size, vec2(0.25, 0.125)));
        let corner = Reference { across: -1, down: -1 };
        let placed = apply_one(mapping, with_point(frame, corner, vec2(0.1, 0.2)));
        assert!(near(placement::corners(&placed)[0], vec2(0.1, 0.2)), "{:?}", placement::corners(&placed));
        let wider = apply_one(placed, resized(frame_of(&placed), corner, vec2(0.5, 0.125)));
        assert!(near(placement::corners(&wider)[0], vec2(0.1, 0.2)) && near(placement::size(&wider), vec2(0.5, 0.125)));
        let turned_label = apply_one(mapping, turned(frame, Reference::default(), 0.5));
        assert!((placement::turn(&turned_label) - 0.5).abs() < 1e-4 && near(placement::centre(&turned_label), vec2(0.5, 0.5)));
    }

    /// Several labels follow the box round them: moved by its corner, and
    /// doubled about it, each keeps its place in the box.
    #[test]
    fn several_labels_follow_their_box() {
        let labels = [label(vec2(0.2, 0.2), vec2(10.0, 10.0)), label(vec2(0.6, 0.4), vec2(5.0, 10.0))];
        let frame = frame_round(&labels).unwrap();
        assert!(near(frame.centre - frame.size * 0.5, vec2(0.15, 0.15)) && near(frame.centre + frame.size * 0.5, vec2(0.7, 0.45)));
        let corner = Reference { across: -1, down: -1 };
        let to = with_point(frame, corner, vec2(0.0, 0.0));
        let moved: Vec<Mapping> = labels.iter().map(|m| apply_group(*m, frame, to, corner)).collect();
        assert!(near(placement::centre(&moved[0]), vec2(0.05, 0.05)) && near(placement::centre(&moved[1]), vec2(0.45, 0.25)));
        let doubled_frame = resized(frame, corner, frame.size * 2.0);
        let doubled: Vec<Mapping> = labels.iter().map(|m| apply_group(*m, frame, doubled_frame, corner)).collect();
        let round = frame_round(&doubled).unwrap();
        assert!(near(round.size, frame.size * 2.0) && near(corner.point(&round), corner.point(&frame)), "{round:?}");
    }

    #[test]
    fn a_typed_field_may_keep_its_name() {
        assert_eq!(prefix_of("X 12 mm"), "X");
        assert_eq!(prefix_of("12 mm"), "");
        assert_eq!(prefix_of("W 3in"), "W");
    }
}
