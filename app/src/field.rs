//! The number field every panel uses in place of egui's slider.
//!
//! One control instead of three. egui's slider is a rail, a number box and a
//! label side by side, which in a 330 px inspector is three things to aim
//! at, a row that cannot line up with the rows around it, and a label that
//! lands after the number it names. This is the field Blender's and most
//! design tools' properties are made of: the name on the left, the value on
//! the right, the proportion of the range as a fill behind them.
//!
//! - Drag across it to change the value; hold Shift for a tenth of the speed.
//! - Click it to type: a number, a number with its unit, or `=` and an
//!   expression ([`crate::numeric::parse`]). Enter or clicking away commits,
//!   Escape cancels.
//!
//! Its builder is the slice of `egui::Slider`'s that the application uses,
//! so switching a call site over is a change of path and nothing else.

use crate::theme;
use eframe::egui;
use std::ops::RangeInclusive;

type GetSet<'a> = Box<dyn 'a + FnMut(Option<f64>) -> f64>;

#[must_use = "add it with `ui.add(...)`"]
pub struct Slider<'a> {
    get_set: GetSet<'a>,
    range: RangeInclusive<f64>,
    integral: bool,
    text: String,
    logarithmic: bool,
    prefix: String,
    suffix: String,
    clamping: egui::SliderClamping,
    show_value: bool,
    step: Option<f64>,
    decimals: Option<usize>,
    unit: Option<crate::numeric::Unit>,
}

impl<'a> Slider<'a> {
    pub fn new<N: egui::emath::Numeric>(value: &'a mut N, range: RangeInclusive<N>) -> Self {
        let integral = N::INTEGRAL;
        let range = range.start().to_f64()..=range.end().to_f64();
        Slider {
            get_set: Box::new(move |set: Option<f64>| {
                if let Some(v) = set {
                    *value = N::from_f64(v);
                }
                value.to_f64()
            }),
            range,
            integral,
            text: String::new(),
            logarithmic: false,
            prefix: String::new(),
            suffix: String::new(),
            clamping: egui::SliderClamping::Always,
            show_value: true,
            step: None,
            decimals: None,
            unit: None,
        }
    }

    /// The name shown inside the field.
    pub fn text(mut self, text: impl Into<egui::WidgetText>) -> Self {
        self.text = text.into().text().to_string();
        self
    }

    /// Spread the range on a log scale, for values that matter in ratios — a
    /// scale from 0.01 to 100 is otherwise all in its last pixel.
    pub fn logarithmic(mut self, logarithmic: bool) -> Self {
        self.logarithmic = logarithmic;
        self
    }

    pub fn suffix(mut self, suffix: impl ToString) -> Self {
        self.suffix = suffix.to_string();
        self
    }

    pub fn prefix(mut self, prefix: impl ToString) -> Self {
        self.prefix = prefix.to_string();
        self
    }

    /// Whether a typed value may leave the range. Dragging never does.
    pub fn clamping(mut self, clamping: egui::SliderClamping) -> Self {
        self.clamping = clamping;
        self
    }

    pub fn show_value(mut self, show: bool) -> Self {
        self.show_value = show;
        self
    }

    /// Land only on multiples of `step`.
    pub fn step_by(mut self, step: f64) -> Self {
        self.step = (step > 0.0).then_some(step);
        self
    }

    /// Show exactly this many decimals.
    /// What a typed number without a unit is in, so a typed length can be
    /// in any unit — `2in` into a field in millimetres.
    pub fn unit(mut self, unit: crate::numeric::Unit) -> Self {
        self.unit = Some(unit);
        self
    }

    pub fn fixed_decimals(mut self, decimals: usize) -> Self {
        self.decimals = Some(decimals);
        self
    }

    fn get(&mut self) -> f64 {
        (self.get_set)(None)
    }

    fn set(&mut self, value: f64) {
        let value = match self.step {
            Some(step) => (value / step).round() * step,
            None => value,
        };
        let value = if self.integral { value.round() } else { value };
        (self.get_set)(Some(value));
    }

    /// Where `value` sits along the field, 0 to 1.
    fn position(&self, value: f64) -> f64 {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        if hi <= lo {
            return 0.0;
        }
        if self.logarithmic && hi > 0.0 {
            let floor = if lo > 0.0 { lo } else { hi * 1e-4 };
            if value <= floor {
                return 0.0;
            }
            return ((value / floor).ln() / (hi / floor).ln()).clamp(0.0, 1.0);
        }
        ((value - lo) / (hi - lo)).clamp(0.0, 1.0)
    }

    /// The value at `t` along the field.
    fn value_at(&self, t: f64) -> f64 {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let t = t.clamp(0.0, 1.0);
        if self.logarithmic && hi > 0.0 && hi > lo {
            let floor = if lo > 0.0 { lo } else { hi * 1e-4 };
            if t <= 0.0 {
                return lo;
            }
            return (floor * (hi / floor).powf(t)).clamp(lo, hi);
        }
        lo + (hi - lo) * t
    }

    /// The value as the field shows it: as many decimals as the range calls
    /// for, more for a small value on a log scale.
    fn format(&self, value: f64) -> String {
        if self.integral {
            return format!("{}{}{}", self.prefix, value.round() as i64, self.suffix);
        }
        if let Some(decimals) = self.decimals {
            return format!("{}{:.*}{}", self.prefix, decimals, value, self.suffix);
        }
        let span = (*self.range.end() - *self.range.start()).abs().max(1e-9);
        let mut decimals = (2.0 - span.log10().floor()).clamp(1.0, 4.0) as usize;
        if self.logarithmic && value.abs() > 0.0 && value.abs() < 1.0 {
            decimals = decimals.max((1.0 - value.abs().log10().floor()).clamp(1.0, 4.0) as usize);
        }
        format!("{}{:.*}{}", self.prefix, decimals, value, self.suffix)
    }
}

/// What a field is doing between frames.
#[derive(Clone, Debug)]
enum State {
    /// Being dragged: where along the field the drag has taken it, kept
    /// unrounded so Shift's fine steps accumulate.
    Dragging(f64),
    /// Being typed into.
    Typing(String),
}

impl egui::Widget for Slider<'_> {
    fn ui(mut self, ui: &mut egui::Ui) -> egui::Response {
        let font = egui::FontId::proportional(theme::BODY - 0.5);
        let label_width = if self.text.is_empty() {
            0.0
        } else {
            ui.painter().layout_no_wrap(self.text.clone(), font.clone(), theme::p().text).size().x
        };
        // Full width down a column; in a row, about the room egui's slider
        // took, so whatever follows it on the row still fits.
        let width = if ui.layout().main_dir().is_horizontal() {
            (ui.spacing().slider_width + 72.0 + label_width).min(ui.available_width())
        } else {
            ui.available_width()
        }
        .max(60.0);
        let height = 26.0;
        let (rect, mut response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
        let id = response.id;
        let state: Option<State> = ui.data(|d| d.get_temp(id));
        let current = self.get();

        // ---- Typing -------------------------------------------------------
        if let Some(State::Typing(mut text)) = state.clone() {
            let edit = ui.put(
                rect,
                egui::TextEdit::singleline(&mut text)
                    .id(id.with("typing"))
                    .font(font.clone())
                    .horizontal_align(egui::Align::RIGHT)
                    .margin(egui::Margin::symmetric(8.0, 4.0)),
            );
            if !edit.has_focus() && !edit.lost_focus() {
                edit.request_focus();
            }
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if escape {
                ui.data_mut(|d| d.remove::<State>(id));
            } else if edit.lost_focus() {
                ui.data_mut(|d| d.remove::<State>(id));
                let bare = text.trim().trim_start_matches(self.prefix.trim()).trim_end_matches(self.suffix.trim());
                if let Some(typed) = crate::numeric::parse(bare, self.unit) {
                    let typed = match self.clamping {
                        egui::SliderClamping::Never => typed,
                        _ => typed.clamp(*self.range.start(), *self.range.end()),
                    };
                    if typed != current {
                        self.set(typed);
                        response.mark_changed();
                    }
                }
            } else {
                ui.data_mut(|d| d.insert_temp(id, State::Typing(text)));
            }
            return response;
        }

        // ---- Dragging and clicking ----------------------------------------
        let mut dragging = false;
        if response.drag_started() {
            ui.data_mut(|d| d.insert_temp(id, State::Dragging(self.position(current))));
        }
        if response.dragged() {
            dragging = true;
            let fine = ui.input(|i| i.modifiers.shift);
            let start = match state {
                Some(State::Dragging(t)) => t,
                _ => self.position(current),
            };
            let t = start + response.drag_delta().x as f64 / rect.width().max(1.0) as f64 * if fine { 0.1 } else { 1.0 };
            let t = t.clamp(0.0, 1.0);
            ui.data_mut(|d| d.insert_temp(id, State::Dragging(t)));
            let next = self.value_at(t);
            let next = match self.step {
                Some(step) => (next / step).round() * step,
                None => next,
            };
            let next = if self.integral { next.round() } else { next };
            if next != current {
                self.set(next);
                response.mark_changed();
            }
        } else if matches!(state, Some(State::Dragging(_))) {
            ui.data_mut(|d| d.remove::<State>(id));
        }
        if response.clicked() && self.show_value {
            let shown = self.format(current);
            let bare = shown.trim_start_matches(&self.prefix).trim_end_matches(&self.suffix).to_string();
            ui.data_mut(|d| d.insert_temp(id, State::Typing(bare)));
            ui.ctx().request_repaint();
        }
        if response.hovered() || dragging {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }

        // ---- Drawing ------------------------------------------------------
        if ui.is_rect_visible(rect) {
            let value = self.get();
            let hovered = response.hovered() || dragging;
            let painter = ui.painter();
            let p = theme::p();
            // The well, and the fill of the range: a neutral step from the
            // well at rest, the accent while it is being dragged.
            let (well, fill) = if dragging {
                (p.field, p.accent.gamma_multiply(if p.dark { 0.45 } else { 0.38 }))
            } else {
                // A step from the well towards the text, so it follows the
                // theme's tint as the well does.
                (well(&p, hovered), theme::mix(p.field, p.text, if hovered { 0.17 } else { 0.13 }))
            };
            painter.rect_filled(rect, 5.0, well);
            let t = self.position(value) as f32;
            if t > 0.0 {
                let filled = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + rect.width() * t, rect.bottom()));
                // Rounded at the left; square where the fill ends, unless it
                // reaches the field's right edge.
                let rounding = egui::Rounding { nw: 5.0, sw: 5.0, ne: if t >= 0.999 { 5.0 } else { 0.0 }, se: if t >= 0.999 { 5.0 } else { 0.0 } };
                painter.rect_filled(filled, rounding, fill);
            }
            let enabled = ui.is_enabled();
            let ink = |c: egui::Color32| if enabled { c } else { c.gamma_multiply(0.45) };
            let inset = 9.0;
            let value_galley = self.show_value.then(|| {
                painter.layout_no_wrap(self.format(value), font.clone(), ink(if hovered { theme::p().icon_hover } else { theme::p().text }))
            });
            let value_width = value_galley.as_ref().map(|g| g.size().x).unwrap_or(0.0);
            if let Some(galley) = value_galley {
                painter.galley(egui::pos2(rect.right() - inset - galley.size().x, rect.center().y - galley.size().y * 0.5), galley, egui::Color32::PLACEHOLDER);
            }
            if !self.text.is_empty() {
                let room = (rect.width() - inset * 2.0 - value_width - 8.0).max(0.0);
                let mut job = egui::text::LayoutJob::simple_singleline(self.text.clone(), font, ink(theme::p().text_dim));
                job.wrap = egui::text::TextWrapping { max_width: room, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                let galley = ui.fonts(|f| f.layout_job(job));
                painter.galley(egui::pos2(rect.left() + inset, rect.center().y - galley.size().y * 0.5), galley, egui::Color32::PLACEHOLDER);
            }
        }
        response
    }
}

/// `add` in a column `width` wide and a field high, laid out down rather
/// than across: a field in a row then takes the whole column, as it would
/// on a row of its own, and the rows of a panel line up.
pub fn column<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let width = width.max(60.0);
    ui.allocate_ui_with_layout(egui::vec2(width, 26.0), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_width(width);
        add(ui)
    })
}

/// The well a field sits in, lifted a little under the pointer.
pub fn well(p: &theme::Palette, hovered: bool) -> egui::Color32 {
    match (hovered, p.dark) {
        (false, _) => p.field,
        (true, true) => p.field.gamma_multiply(1.15),
        (true, false) => p.control,
    }
}

/// A tick box, in the accent when ticked: egui's own draws its tick on a
/// tint of the selection colour, which on a dark panel reads as no box at
/// all.
#[must_use = "add it with `ui.add(...)`"]
pub struct Checkbox<'a> {
    checked: &'a mut bool,
    text: egui::WidgetText,
}

impl<'a> Checkbox<'a> {
    pub fn new(checked: &'a mut bool, text: impl Into<egui::WidgetText>) -> Self {
        Checkbox { checked, text: text.into() }
    }
}

impl egui::Widget for Checkbox<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let Checkbox { checked, text } = self;
        let box_side = 16.0;
        let gap = 8.0;
        let galley = (!text.is_empty()).then(|| {
            text.into_galley(ui, None, ui.available_width() - box_side - gap, egui::TextStyle::Button)
        });
        let text_size = galley.as_ref().map(|g| g.size()).unwrap_or_default();
        let size = egui::vec2(
            box_side + if galley.is_some() { gap + text_size.x } else { 0.0 },
            text_size.y.max(box_side).max(ui.spacing().interact_size.y - 4.0),
        );
        let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
        if response.clicked() {
            *checked = !*checked;
            response.mark_changed();
        }
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *checked, galley.as_ref().map(|g| g.text()).unwrap_or("")));
        if ui.is_rect_visible(rect) {
            let enabled = ui.is_enabled();
            let ink = |c: egui::Color32| if enabled { c } else { c.gamma_multiply(0.45) };
            let hovered = response.hovered();
            let square = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - box_side * 0.5), egui::Vec2::splat(box_side));
            let painter = ui.painter();
            if *checked {
                painter.rect_filled(square, 5.0, ink(if hovered { theme::p().accent_text } else { theme::p().accent }));
                let p = |x: f32, y: f32| square.min + egui::vec2(x, y) * (box_side / 16.0);
                painter.add(egui::Shape::line(vec![p(4.0, 8.4), p(6.9, 11.2), p(12.0, 5.2)], egui::Stroke::new(2.0_f32, ink(theme::p().on_accent))));
            } else {
                let p = theme::p();
                painter.rect(
                    square,
                    5.0,
                    ink(p.field),
                    egui::Stroke::new(1.2_f32, ink(if hovered { p.accent } else { p.line_strong })),
                );
            }
            if let Some(galley) = galley {
                let colour = ink(if hovered { theme::p().icon_hover } else { theme::p().text });
                painter.galley(egui::pos2(square.right() + gap, rect.center().y - galley.size().y * 0.5), galley, colour);
            }
        }
        response
    }
}

thread_local! {
    /// How many folds deep the current body is: an outer fold is a card, a
    /// fold inside one is a plain row, so cards never stack in cards.
    static FOLD_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A group of controls that opens and closes: a card with its title on a
/// row that can be clicked anywhere. In place of egui's collapsing header,
/// with the slice of its builder the application uses.
#[must_use = "show it with `.show(ui, ...)`"]
pub struct Fold {
    title: egui::WidgetText,
    id_salt: Option<egui::Id>,
    default_open: bool,
}

impl Fold {
    pub fn new(title: impl Into<egui::WidgetText>) -> Self {
        Fold { title: title.into(), id_salt: None, default_open: false }
    }

    pub fn id_salt(mut self, salt: impl std::hash::Hash) -> Self {
        self.id_salt = Some(egui::Id::new(salt));
        self
    }

    pub fn default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    pub fn show<R>(self, ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::CollapsingResponse<R> {
        let p = theme::p();
        let title = self.title.text().to_string();
        let id = ui.make_persistent_id(self.id_salt.unwrap_or_else(|| egui::Id::new(&title)));
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, self.default_open);
        let nested = FOLD_DEPTH.with(|d| d.get()) > 0;
        let frame = if nested {
            egui::Frame::none()
        } else {
            egui::Frame::none()
                .fill(p.card)
                .stroke(if p.dark { egui::Stroke::NONE } else { egui::Stroke::new(1.0_f32, p.line) })
                .rounding(10.0)
                .inner_margin(egui::Margin { left: 6.0, right: 8.0, top: 3.0, bottom: 3.0 })
        };
        let inner = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (rect, header) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
            if header.clicked() {
                state.toggle(ui);
            }
            let open = state.is_open();
            if ui.is_rect_visible(rect) {
                let painter = ui.painter();
                if header.hovered() {
                    painter.rect_filled(rect, 7.0, p.hover.gamma_multiply(0.5));
                }
                let chevron = egui::Rect::from_center_size(egui::pos2(rect.left() + 12.0, rect.center().y), egui::Vec2::splat(12.0));
                crate::icons::paint(
                    painter,
                    chevron,
                    if open { crate::icons::Icon::ChevronDown } else { crate::icons::Icon::ChevronRight },
                    if header.hovered() { p.text } else { p.text_dim },
                );
                let galley = painter.layout_no_wrap(title.clone(), theme::strong(if nested { 12.5 } else { 13.0 }), p.text);
                painter.galley(egui::pos2(rect.left() + 26.0, rect.center().y - galley.size().y * 0.5), galley, egui::Color32::PLACEHOLDER);
            }
            let body = state.show_body_unindented(ui, |ui| {
                FOLD_DEPTH.with(|d| d.set(d.get() + 1));
                let returned = egui::Frame::none().inner_margin(egui::Margin { left: if nested { 18.0 } else { 6.0 }, right: 0.0, top: 2.0, bottom: 8.0 }).show(ui, add).inner;
                FOLD_DEPTH.with(|d| d.set(d.get() - 1));
                returned
            });
            (header, body)
        });
        if !nested {
            ui.add_space(2.0);
        }
        let (header_response, body) = inner.inner;
        egui::CollapsingResponse {
            header_response,
            openness: state.openness(ui.ctx()),
            body_response: body.as_ref().map(|b| b.response.clone()),
            body_returned: body.map(|b| b.inner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Harness {
        ctx: egui::Context,
        time: f64,
    }

    impl Harness {
        fn new() -> Harness {
            Harness { ctx: egui::Context::default(), time: 0.0 }
        }

        /// One frame with `field` in a 300-point column, and its rect.
        fn frame(&mut self, events: Vec<egui::Event>, field: impl FnOnce(&mut egui::Ui) -> egui::Response) -> egui::Response {
            self.time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (mut out, mut field) = (None, Some(field));
            let _ = self.ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(field) = field.take() {
                        out = Some(field(ui));
                    }
                });
            });
            out.unwrap()
        }
    }

    fn press(at: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
        ]
    }

    /// Dragging across the whole field takes the value across the whole
    /// range, and no further.
    #[test]
    fn a_drag_moves_through_the_range_and_stops_at_its_end() {
        let mut h = Harness::new();
        let mut value = 0.25_f32;
        let rect = h.frame(Vec::new(), |ui| ui.add(Slider::new(&mut value, 0.0..=1.0).text("Roughness"))).rect;
        let start = egui::pos2(rect.left() + rect.width() * 0.25, rect.center().y);
        h.frame(press(start, true), |ui| ui.add(Slider::new(&mut value, 0.0..=1.0)));
        let mut changed = false;
        for step in 1..=10 {
            let at = start + egui::vec2(rect.width() * 0.05 * step as f32, 0.0);
            changed |= h.frame(vec![egui::Event::PointerMoved(at)], |ui| ui.add(Slider::new(&mut value, 0.0..=1.0))).changed();
        }
        assert!(changed, "a drag reports a change");
        assert!((value - 0.75).abs() < 0.02, "half the width is half the range: {value}");
        for step in 1..=10 {
            let at = start + egui::vec2(rect.width() * (0.5 + 0.2 * step as f32), 0.0);
            h.frame(vec![egui::Event::PointerMoved(at)], |ui| ui.add(Slider::new(&mut value, 0.0..=1.0)));
        }
        assert_eq!(value, 1.0, "and not past the end");
        h.frame(press(start, false), |ui| ui.add(Slider::new(&mut value, 0.0..=1.0)));
    }

    /// A click opens the field for typing, and what is typed is the value —
    /// clamped to the range unless the field says otherwise.
    #[test]
    fn a_click_types_a_value() {
        let mut h = Harness::new();
        let mut value = 3_u32;
        let rect = h.frame(Vec::new(), |ui| ui.add(Slider::new(&mut value, 1..=10).text("Detail"))).rect;
        h.frame(press(rect.center(), true), |ui| ui.add(Slider::new(&mut value, 1..=10)));
        h.frame(press(rect.center(), false), |ui| ui.add(Slider::new(&mut value, 1..=10)));
        h.frame(Vec::new(), |ui| ui.add(Slider::new(&mut value, 1..=10)));
        h.frame(
            vec![
                egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND },
                egui::Event::Text("=2*3".into()),
            ],
            |ui| ui.add(Slider::new(&mut value, 1..=10)),
        );
        let response = h.frame(
            vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
            |ui| ui.add(Slider::new(&mut value, 1..=10)),
        );
        assert_eq!(value, 6, "the expression was evaluated and set");
        assert!(response.changed());
    }

    /// A field with a unit takes a length typed in another: the Camera's
    /// focal length is in millimetres, and `2in` is 50.8 of them.
    #[test]
    fn a_field_with_a_unit_takes_any_unit() {
        let mut h = Harness::new();
        let mut value = 50.0_f32;
        fn field(value: &mut f32) -> Slider<'_> {
            Slider::new(value, 8.0..=400.0).suffix(" mm").unit(crate::numeric::Unit::Millimetres)
        }
        let rect = h.frame(Vec::new(), |ui| ui.add(field(&mut value))).rect;
        h.frame(press(rect.center(), true), |ui| ui.add(field(&mut value)));
        h.frame(press(rect.center(), false), |ui| ui.add(field(&mut value)));
        h.frame(Vec::new(), |ui| ui.add(field(&mut value)));
        h.frame(
            vec![
                egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND },
                egui::Event::Text("2in".into()),
            ],
            |ui| ui.add(field(&mut value)),
        );
        h.frame(
            vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
            |ui| ui.add(field(&mut value)),
        );
        assert!((value - 50.8).abs() < 1e-3, "two inches is 50.8 mm: {value}");
    }

    /// A log field spends its width evenly on ratios, and finds its way back
    /// to the value it was given.
    #[test]
    fn a_logarithmic_field_round_trips() {
        let mut v = 1.0_f64;
        let slider = Slider::new(&mut v, 0.01..=100.0).logarithmic(true);
        assert!((slider.position(1.0) - 0.5).abs() < 1e-9, "1 is halfway between 0.01 and 100");
        for value in [0.01, 0.05, 1.0, 7.5, 100.0] {
            assert!((slider.value_at(slider.position(value)) - value).abs() < 1e-9 * value.max(1.0));
        }
        assert_eq!(slider.format(0.005), "0.0050", "small values keep their digits");
    }
}
