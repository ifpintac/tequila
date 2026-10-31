//! The rulers along the canvas's top and left edges: ticks that keep a
//! readable spacing at any zoom, in the part's millimetres, centimetres or
//! inches where the layout has one size, and in layout units where it has
//! not; and where the pointer is, marked on both. Guides are dragged out of
//! them (`super::guides`).

use super::units::{Scale, Unit};
use super::view::Xform;

/// How thick a ruler is, in points.
pub const THICKNESS: f32 = 20.0;

/// The least room between labelled ticks, and between the small ones.
const MAJOR_PIXELS: f64 = 64.0;
const MINOR_PIXELS: f64 = 6.0;

/// Where a ruler's ticks fall: a labelled one every `major`, in the unit
/// shown, and `minor` divisions between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ticks {
    pub major: f64,
    pub minor: u32,
}

/// The ticks for a ruler showing `pixels_per_unit` screen points to one of
/// its unit: labelled ticks on a 1, 2, 5 series - or, in inches, halves,
/// quarters, eighths and sixteenths below an inch - the closest of them at
/// least `MAJOR_PIXELS` apart, and as many divisions between as stay apart.
pub fn ticks(pixels_per_unit: f64, inches: bool) -> Ticks {
    let pixels_per_unit = pixels_per_unit.max(1e-9);
    let wanted = MAJOR_PIXELS / pixels_per_unit;
    let mut major = None;
    if inches && wanted <= 1.0 {
        for step in [1.0 / 16.0, 1.0 / 8.0, 0.25, 0.5, 1.0] {
            if step >= wanted {
                major = Some(step);
                break;
            }
        }
    }
    let major = major.unwrap_or_else(|| {
        let power = 10f64.powf(wanted.log10().floor());
        [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * power).find(|step| *step >= wanted * (1.0 - 1e-9)).unwrap_or(10.0 * power)
    });
    // Divisions in the series the step belongs to: tenths, fifths or
    // halves of a 1; quarters or halves of a 2; fifths of a 5; and in
    // inches below ten, binary fractions.
    let leading = (major / 10f64.powf(major.log10().floor())).round() as u32;
    let candidates: &[u32] = if inches && major <= 1.0 + 1e-9 {
        &[16, 8, 4, 2]
    } else {
        match leading {
            1 => &[10, 5, 2],
            2 => &[4, 2],
            _ => &[5],
        }
    };
    let minor = candidates.iter().copied().find(|n| major * pixels_per_unit / *n as f64 >= MINOR_PIXELS).unwrap_or(1);
    Ticks { major, minor }
}

/// A tick's number, to as few decimals as its step needs.
pub fn label(value: f64, step: f64) -> String {
    let mut decimals = 0;
    while decimals < 4 && ((step * 10f64.powi(decimals)).round() - step * 10f64.powi(decimals)).abs() > 1e-6 {
        decimals += 1;
    }
    let value = if value.abs() < step * 1e-6 { 0.0 } else { value };
    format!("{:.*}", decimals as usize, value)
}

/// Draw a ruler: along the top (`across`) or down the left, over `rect`,
/// reading the layout through `xform` in `scale`, with the pointer's place
/// on it marked.
pub fn draw(painter: &egui::Painter, rect: egui::Rect, xform: &Xform, scale: Scale, across: bool, pointer: Option<f32>) {
    let p = crate::theme::p();
    painter.rect_filled(rect, 0.0, p.chrome);
    let edge = if across {
        [rect.left_bottom(), rect.right_bottom()]
    } else {
        [rect.right_top(), rect.right_bottom()]
    };
    painter.line_segment(edge, egui::Stroke::new(1.0_f32, p.line));
    let per_point = xform.scale as f64 * 1.0 / scale.per_uv as f64;
    let Ticks { major, minor } = ticks(per_point, scale.unit == Unit::Inches);
    // The span of the ruler in its unit.
    let (start, end) = if across {
        (xform.to_layout(rect.left_top()).x, xform.to_layout(rect.right_top()).x)
    } else {
        (xform.to_layout(rect.left_top()).y, xform.to_layout(rect.left_bottom()).y)
    };
    let (start, end) = (scale.shown(start) as f64, scale.shown(end) as f64);
    let at = |value: f64| -> f32 {
        let uv = (value / scale.per_uv as f64) as f32;
        if across {
            xform.origin.x + uv * xform.scale
        } else {
            xform.origin.y + uv * xform.scale
        }
    };
    let stroke = egui::Stroke::new(1.0_f32, p.text_faint);
    let font = egui::FontId::proportional(10.0);
    let first = (start / major).floor() as i64;
    let last = (end / major).ceil() as i64;
    // A guard against a span so wide it would be thousands of ticks.
    if last - first > 400 {
        return;
    }
    for k in first..=last {
        let value = k as f64 * major;
        let place = at(value);
        let (long, short) = (THICKNESS * 0.75, THICKNESS * 0.25);
        if across {
            painter.line_segment([egui::pos2(place, rect.bottom() - long), egui::pos2(place, rect.bottom())], stroke);
            painter.text(egui::pos2(place + 3.0, rect.top() + 1.0), egui::Align2::LEFT_TOP, label(value, major), font.clone(), p.text_dim);
        } else {
            painter.line_segment([egui::pos2(rect.right() - long, place), egui::pos2(rect.right(), place)], stroke);
            let galley = painter.layout_no_wrap(label(value, major), font.clone(), p.text_dim);
            // Turned to read up the ruler, as a drawing's side ruler does.
            let text = egui::epaint::TextShape::new(egui::pos2(rect.left() + 2.0, place - 3.0), galley, p.text_dim)
                .with_angle(-std::f32::consts::FRAC_PI_2);
            painter.add(text);
        }
        for i in 1..minor {
            let place = at(value + major * i as f64 / minor as f64);
            let length = if minor % 2 == 0 && i * 2 == minor { short * 1.6 } else { short };
            if across {
                painter.line_segment([egui::pos2(place, rect.bottom() - length), egui::pos2(place, rect.bottom())], stroke);
            } else {
                painter.line_segment([egui::pos2(rect.right() - length, place), egui::pos2(rect.right(), place)], stroke);
            }
        }
    }
    if let Some(uv) = pointer {
        let place = if across { xform.origin.x + uv * xform.scale } else { xform.origin.y + uv * xform.scale };
        let marker = egui::Stroke::new(1.0_f32, p.accent);
        if across {
            painter.line_segment([egui::pos2(place, rect.top()), egui::pos2(place, rect.bottom())], marker);
        } else {
            painter.line_segment([egui::pos2(rect.left(), place), egui::pos2(rect.right(), place)], marker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The labelled ticks are the nearest step of the series at least
    /// `MAJOR_PIXELS` apart, and the small ones divide them as finely as
    /// stays readable.
    #[test]
    fn the_ticks_keep_their_spacing_at_any_zoom() {
        // Ten points a millimetre: a label every ten, a tick every one.
        assert_eq!(ticks(10.0, false), Ticks { major: 10.0, minor: 10 });
        // One point a millimetre: every hundred, ticks every ten.
        assert_eq!(ticks(1.0, false), Ticks { major: 100.0, minor: 10 });
        // Forty points a millimetre: every two, in quarters, twenty points
        // apart.
        assert_eq!(ticks(40.0, false), Ticks { major: 2.0, minor: 4 });
        // Far in on the layout's own units: thousandths.
        let fine = ticks(100_000.0, false);
        assert!((fine.major - 0.001).abs() < 1e-12, "{fine:?}");
        // Far out: a 5 step, in fifths.
        assert_eq!(ticks(0.2, false), Ticks { major: 500.0, minor: 5 });
        for zoom in [0.013, 0.7, 3.3, 47.0, 980.0, 12_345.0] {
            let t = ticks(zoom, false);
            assert!(t.major * zoom >= MAJOR_PIXELS - 1e-6, "{zoom}: {t:?}");
            assert!(t.major * zoom < MAJOR_PIXELS * 2.5 + 1e-6, "{zoom}: {t:?} too sparse");
            assert!(t.minor == 1 || t.major * zoom / t.minor as f64 >= MINOR_PIXELS, "{zoom}: {t:?}");
        }
    }

    /// Inches below an inch go by halves, quarters, eighths and sixteenths,
    /// and divide as a tape measure does.
    #[test]
    fn inches_go_by_binary_fractions() {
        assert_eq!(ticks(300.0, true), Ticks { major: 0.25, minor: 8 });
        assert_eq!(ticks(100.0, true), Ticks { major: 1.0, minor: 16 });
        assert_eq!(ticks(30.0, true), Ticks { major: 5.0, minor: 5 });
    }

    #[test]
    fn a_tick_s_number_has_the_decimals_its_step_needs() {
        assert_eq!(label(20.0, 10.0), "20");
        assert_eq!(label(0.25, 0.25), "0.25");
        assert_eq!(label(0.002, 0.001), "0.002");
        assert_eq!(label(-1e-12, 0.5), "0.0");
        assert_eq!(label(1.5, 0.5), "1.5");
    }
}
