//! Typing a number into a field (`docs/04-UX-SPEC.md` §1.2).
//!
//! > Every numeric field supports drag-to-scrub, type-to-enter, arrow-key
//! > increment (`Shift` = ×10, `Ctrl` = ×0.1), unit suffixes (`85mm`,
//! > `2.5in`), and simple expression entry (`= 36/1.5`).
//!
//! egui gives the first two and the third. This is the other two, and they
//! matter more than they look. A person working from a drawing has the number
//! in the drawing's units, not in the scene's, and the alternative to typing
//! `2.5in` is doing the conversion in their head — which is where the mistakes
//! come from. The expression is the same argument one step further: `= 36/1.5`
//! is a sensor width divided by a crop factor, and it is worth more written
//! down than resolved to `24` and forgotten.
//!
//! # What it deliberately does not do
//!
//! There is no variable lookup, no function library and no operator the four
//! on a pocket calculator do not cover. A field that evaluates arbitrary
//! expressions is a field somebody has to debug, and the value of this feature
//! is entirely in the ten seconds it saves — a feature that can go wrong in
//! interesting ways has already spent that.

use eframe::egui;

/// Length units a field may be typed in.
///
/// The scene has its own unit (`docs/03` §2) and every field is *in* that unit.
/// A suffix says "I am giving you this number in a different unit; convert it".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Millimetres,
    Centimetres,
    Metres,
    Inches,
    Feet,
}

impl Unit {
    /// Metres per one of these.
    fn in_metres(self) -> f64 {
        match self {
            Unit::Millimetres => 0.001,
            Unit::Centimetres => 0.01,
            Unit::Metres => 1.0,
            Unit::Inches => 0.0254,
            Unit::Feet => 0.3048,
        }
    }

    /// Parse a suffix. Longest first, so `mm` is not read as `m` with a
    /// trailing `m`.
    fn from_suffix(text: &str) -> Option<Unit> {
        match text.trim().to_ascii_lowercase().as_str() {
            "mm" => Some(Unit::Millimetres),
            "cm" => Some(Unit::Centimetres),
            "m" => Some(Unit::Metres),
            "in" | "\"" => Some(Unit::Inches),
            "ft" | "'" => Some(Unit::Feet),
            _ => None,
        }
    }

    /// The scene's own unit, when it has one. `Unspecified` means the document
    /// has not established a scale, and converting into a unit nobody has
    /// declared would be inventing one.
    pub fn from_scene(unit: tequila_core::units::LengthUnit) -> Option<Unit> {
        use tequila_core::units::LengthUnit as L;
        match unit {
            L::Unspecified => None,
            L::Millimetre => Some(Unit::Millimetres),
            L::Centimetre => Some(Unit::Centimetres),
            L::Metre => Some(Unit::Metres),
            L::Inch => Some(Unit::Inches),
            L::Foot => Some(Unit::Feet),
        }
    }
}

/// A `DragValue` that understands §1.2's units and expressions.
///
/// Wrapping the parser into the widget rather than leaving it to each call
/// site, because a field that accepts `85mm` in one panel and not in the next
/// is worse than one that never did — the user has to remember which is which.
pub fn drag_value(value: &mut f32, unit: Option<Unit>) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .custom_parser(move |text| parse(text, unit))
        .custom_formatter(|v, range| format(v, (*range.end()).min(4)))
}

/// Read a field's text.
///
/// `field_unit` is the unit the field's number is in — the scene's unit for a
/// position, millimetres for a focal length. `None` means the field is not a
/// length at all (a count, a percentage, an angle), and a unit suffix on one
/// of those is a typo rather than a conversion.
pub fn parse(text: &str, field_unit: Option<Unit>) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    // An expression is anything introduced by `=`. Requiring the `=` is what
    // keeps `1-2` — which somebody typing a negative number gets to halfway
    // through — from silently evaluating to -1 as they type.
    if let Some(rest) = text.strip_prefix('=') {
        return evaluate(rest);
    }

    // Split the trailing letters (or a quote mark) off the number.
    let split = text
        .rfind(|c: char| c.is_ascii_digit() || c == '.')
        .map(|i| i + 1)
        .unwrap_or(0);
    let (number, suffix) = text.split_at(split);
    let value: f64 = number.trim().parse().ok()?;

    if suffix.trim().is_empty() {
        return Some(value);
    }
    let typed = Unit::from_suffix(suffix)?;
    let field = field_unit?;
    Some(value * typed.in_metres() / field.in_metres())
}

/// A four-operation expression with parentheses, left to right with the usual
/// precedence.
fn evaluate(text: &str) -> Option<f64> {
    let tokens: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let mut at = 0usize;
    let value = sum(&tokens, &mut at)?;
    // Trailing rubbish is a typo, and quietly ignoring it would evaluate
    // `= 3 + ` to 3 while the user is still typing the second operand.
    (at == tokens.len()).then_some(value)
}

fn sum(t: &[char], at: &mut usize) -> Option<f64> {
    let mut value = product(t, at)?;
    while let Some(&c) = t.get(*at) {
        if c != '+' && c != '-' {
            break;
        }
        *at += 1;
        let rhs = product(t, at)?;
        value = if c == '+' { value + rhs } else { value - rhs };
    }
    Some(value)
}

fn product(t: &[char], at: &mut usize) -> Option<f64> {
    let mut value = unary(t, at)?;
    while let Some(&c) = t.get(*at) {
        if c != '*' && c != '/' {
            break;
        }
        *at += 1;
        let rhs = unary(t, at)?;
        if c == '*' {
            value *= rhs;
        } else {
            // Division by zero yields no value rather than an infinity. An
            // infinity in a transform propagates into the BVH and takes the
            // whole scene with it.
            if rhs == 0.0 {
                return None;
            }
            value /= rhs;
        }
    }
    Some(value)
}

fn unary(t: &[char], at: &mut usize) -> Option<f64> {
    match t.get(*at) {
        Some('-') => {
            *at += 1;
            unary(t, at).map(|v| -v)
        }
        Some('+') => {
            *at += 1;
            unary(t, at)
        }
        _ => atom(t, at),
    }
}

fn atom(t: &[char], at: &mut usize) -> Option<f64> {
    if t.get(*at) == Some(&'(') {
        *at += 1;
        let value = sum(t, at)?;
        if t.get(*at) != Some(&')') {
            return None;
        }
        *at += 1;
        return Some(value);
    }
    let start = *at;
    while t
        .get(*at)
        .is_some_and(|c| c.is_ascii_digit() || *c == '.')
    {
        *at += 1;
    }
    if *at == start {
        return None;
    }
    t[start..*at].iter().collect::<String>().parse().ok()
}

/// Format a value back into a field, at a sensible number of decimals.
///
/// Trailing zeros are trimmed. A position field reading `0.5000` invites the
/// belief that four decimals are meaningful, and the ones that are meaningful
/// are then harder to see.
pub fn format(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    if !text.contains('.') {
        return text;
    }
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|v| (v - b).abs() < 1e-9)
    }

    #[test]
    fn a_plain_number_is_itself() {
        assert!(close(parse("42", None), 42.0));
        assert!(close(parse("-3.5", None), -3.5));
        assert!(close(parse("  0.25  ", Some(Unit::Metres)), 0.25));
    }

    /// The conversion is *into* the field's unit, which is the direction that
    /// is easy to get backwards: typing `25.4mm` into a field measured in
    /// inches must give 1, not 645.
    #[test]
    fn a_suffix_converts_into_the_fields_unit() {
        assert!(close(parse("25.4mm", Some(Unit::Inches)), 1.0));
        assert!(close(parse("1in", Some(Unit::Millimetres)), 25.4));
        assert!(close(parse("2.5in", Some(Unit::Centimetres)), 6.35));
        assert!(close(parse("1ft", Some(Unit::Inches)), 12.0));
        assert!(close(parse("1m", Some(Unit::Millimetres)), 1000.0));
    }

    /// `mm` must not be read as `m` followed by junk, which is the mistake a
    /// shortest-match parser makes and which is out by a factor of a thousand.
    #[test]
    fn millimetres_are_not_metres() {
        let mm = parse("5mm", Some(Unit::Metres));
        let m = parse("5m", Some(Unit::Metres));
        assert!(close(mm, 0.005));
        assert!(close(m, 5.0));
    }

    /// The typographer's marks people actually type.
    #[test]
    fn quote_marks_are_units() {
        assert!(close(parse("6\"", Some(Unit::Inches)), 6.0));
        assert!(close(parse("2'", Some(Unit::Inches)), 24.0));
    }

    /// A unit on a field that is not a length is a typo, and rejecting it
    /// leaves the old value in place. Accepting it would silently multiply a
    /// sample count by 0.001.
    #[test]
    fn a_unit_on_a_unitless_field_is_rejected() {
        assert_eq!(parse("64mm", None), None);
        assert!(close(parse("64", None), 64.0));
    }

    #[test]
    fn expressions_follow_the_usual_precedence() {
        assert!(close(parse("= 36/1.5", None), 24.0));
        assert!(close(parse("=2+3*4", None), 14.0));
        assert!(close(parse("=(2+3)*4", None), 20.0));
        assert!(close(parse("=-2*-3", None), 6.0));
        assert!(close(parse("= 10 - 2 - 3", None), 5.0));
    }

    /// The `=` is required, and that is a deliberate constraint rather than
    /// syntax for its own sake. Somebody typing the negative number `-5` passes
    /// through the intermediate text `-`; somebody typing `1.5` passes through
    /// `1.`. A field that evaluated everything would be evaluating half-typed
    /// input on every keystroke.
    #[test]
    fn arithmetic_without_an_equals_sign_is_not_evaluated() {
        assert_eq!(parse("36/1.5", None), None);
        assert_eq!(parse("2+3", None), None);
    }

    /// A half-typed expression yields nothing, so the field keeps its old
    /// value rather than jumping to a partial answer mid-keystroke.
    #[test]
    fn an_incomplete_expression_yields_nothing() {
        assert_eq!(parse("= 3 +", None), None);
        assert_eq!(parse("= (2+3", None), None);
        assert_eq!(parse("=", None), None);
        assert_eq!(parse("= 2 + + ", None), None);
    }

    /// Division by zero yields nothing rather than an infinity. An infinity in
    /// a transform propagates into the BVH and takes the scene with it, and
    /// the field is the cheapest place to stop it.
    #[test]
    fn division_by_zero_is_refused() {
        assert_eq!(parse("= 1/0", None), None);
        assert_eq!(parse("= 5/(3-3)", None), None);
    }

    #[test]
    fn nonsense_is_refused() {
        assert_eq!(parse("", None), None);
        assert_eq!(parse("abc", None), None);
        assert_eq!(parse("12ab", Some(Unit::Metres)), None);
        assert_eq!(parse("--", None), None);
    }

    #[test]
    fn formatting_trims_noise() {
        assert_eq!(format(0.5, 4), "0.5");
        assert_eq!(format(1.0, 4), "1");
        assert_eq!(format(1.2345678, 4), "1.2346");
        assert_eq!(format(-0.0001, 4), "-0.0001");
        assert_eq!(format(100.0, 2), "100");
    }
}
