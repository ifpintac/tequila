//! Align and distribute: labels lined up by their edges or middles, against
//! each other, an island or the layout's square, and spread so the gaps
//! between them are equal. Worked on the boxes square to the layout that
//! hold each label, as turned; what comes back is how far to move each.

use tequila_core::math::{vec2, Vec2};

/// A box square to the layout: its low and high corners.
pub type Box2 = (Vec2, Vec2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Centre,
    Right,
    Top,
    Middle,
    Bottom,
}

impl Align {
    pub const ALL: [Align; 6] = [Align::Left, Align::Centre, Align::Right, Align::Top, Align::Middle, Align::Bottom];

    pub fn icon(self) -> crate::icons::Icon {
        use crate::icons::Icon;
        match self {
            Align::Left => Icon::AlignLeft,
            Align::Centre => Icon::AlignCentre,
            Align::Right => Icon::AlignRight,
            Align::Top => Icon::AlignTop,
            Align::Middle => Icon::AlignMiddle,
            Align::Bottom => Icon::AlignBottom,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Align::Left => "Align left edges",
            Align::Centre => "Align centres",
            Align::Right => "Align right edges",
            Align::Top => "Align top edges",
            Align::Middle => "Align middles",
            Align::Bottom => "Align bottom edges",
        }
    }

    /// Its name on the scripting channel.
    pub fn key(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Centre => "centre",
            Align::Right => "right",
            Align::Top => "top",
            Align::Middle => "middle",
            Align::Bottom => "bottom",
        }
    }

    pub fn from_key(key: &str) -> Option<Align> {
        match key {
            "center" => Some(Align::Centre),
            _ => Align::ALL.into_iter().find(|a| a.key() == key),
        }
    }
}

/// What to line up against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Target {
    /// The box holding everything selected.
    #[default]
    Selection,
    /// The island last clicked.
    Island,
    /// The layout's unit square.
    Layout,
}

impl Target {
    pub fn label(self) -> &'static str {
        match self {
            Target::Selection => "Selection",
            Target::Island => "Island",
            Target::Layout => "Layout",
        }
    }
}

/// The box holding all of `boxes`.
pub fn union(boxes: &[Box2]) -> Option<Box2> {
    boxes.iter().copied().reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
}

/// How far to move each box to line it up with `target` as `how` says.
pub fn align(boxes: &[Box2], target: Box2, how: Align) -> Vec<Vec2> {
    let middle = (target.0 + target.1) * 0.5;
    boxes
        .iter()
        .map(|&(low, high)| {
            let centre = (low + high) * 0.5;
            match how {
                Align::Left => vec2(target.0.x - low.x, 0.0),
                Align::Centre => vec2(middle.x - centre.x, 0.0),
                Align::Right => vec2(target.1.x - high.x, 0.0),
                Align::Top => vec2(0.0, target.0.y - low.y),
                Align::Middle => vec2(0.0, middle.y - centre.y),
                Align::Bottom => vec2(0.0, target.1.y - high.y),
            }
        })
        .collect()
}

/// How far to move each box so that, across (`across`) or down, the gaps
/// between neighbours are equal, the first and the last staying where they
/// are. Neighbours are by where each box's middle is. Fewer than three
/// boxes have nothing to spread.
pub fn distribute(boxes: &[Box2], across: bool) -> Vec<Vec2> {
    let mut moves = vec![Vec2::ZERO; boxes.len()];
    if boxes.len() < 3 {
        return moves;
    }
    let along = |v: Vec2| if across { v.x } else { v.y };
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| along(boxes[a].0 + boxes[a].1).total_cmp(&along(boxes[b].0 + boxes[b].1)));
    let first = boxes[order[0]];
    let last = boxes[*order.last().unwrap()];
    let span = along(last.1) - along(first.0);
    let widths: f32 = order.iter().map(|&i| along(boxes[i].1) - along(boxes[i].0)).sum();
    let gap = (span - widths) / (boxes.len() - 1) as f32;
    let mut at = along(first.0);
    for &i in &order {
        let (low, high) = boxes[i];
        let shift = at - along(low);
        moves[i] = if across { vec2(shift, 0.0) } else { vec2(0.0, shift) };
        at += along(high) - along(low) + gap;
    }
    moves
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-5
    }

    fn boxes() -> Vec<Box2> {
        vec![
            (vec2(0.1, 0.1), vec2(0.2, 0.3)),
            (vec2(0.5, 0.2), vec2(0.9, 0.25)),
            (vec2(0.25, 0.6), vec2(0.35, 0.8)),
        ]
    }

    #[test]
    fn labels_line_up_by_their_edges_and_middles() {
        let all = union(&boxes()).unwrap();
        assert!(near(all.0, vec2(0.1, 0.1)) && near(all.1, vec2(0.9, 0.8)));
        let left = align(&boxes(), all, Align::Left);
        assert!(near(left[0], Vec2::ZERO) && near(left[1], vec2(-0.4, 0.0)) && near(left[2], vec2(-0.15, 0.0)));
        let right = align(&boxes(), all, Align::Right);
        assert!(near(right[0], vec2(0.7, 0.0)) && near(right[1], Vec2::ZERO));
        let middle = align(&boxes(), all, Align::Middle);
        assert!(near(middle[0], vec2(0.0, 0.25)), "{:?}", middle[0]);
        // Against the layout's square, one label goes to its middle.
        let square = (Vec2::ZERO, Vec2::splat(1.0));
        let centred = align(&boxes()[..1], square, Align::Centre);
        assert!(near(centred[0], vec2(0.35, 0.0)));
        let bottom = align(&boxes()[..1], square, Align::Bottom);
        assert!(near(bottom[0], vec2(0.0, 0.7)));
    }

    /// Spread across, the gaps between the three are equal and the ends
    /// stay put, whatever order they were selected in.
    #[test]
    fn labels_spread_with_equal_gaps() {
        let moves = distribute(&boxes(), true);
        let moved: Vec<Box2> = boxes().iter().zip(&moves).map(|(&(a, b), d)| (a + *d, b + *d)).collect();
        assert!(near(moves[0], Vec2::ZERO) && near(moves[1], Vec2::ZERO), "the ends stay: {moves:?}");
        assert!(near(moves[2], vec2(0.05, 0.0)), "the middle one moves to even the gaps: {moves:?}");
        // Widths 0.1, 0.1 and 0.4 over a span of 0.8: gaps of 0.1.
        let mut order: Vec<&Box2> = moved.iter().collect();
        order.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
        for pair in order.windows(2) {
            assert!(((pair[1].0.x - pair[0].1.x) - 0.1).abs() < 1e-5, "{order:?}");
        }
        assert!(moves.iter().all(|m| m.y == 0.0));
        let down = distribute(&boxes(), false);
        let moved: Vec<Box2> = boxes().iter().zip(&down).map(|(&(a, b), d)| (a + *d, b + *d)).collect();
        let mut order: Vec<&Box2> = moved.iter().collect();
        order.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
        let gaps: Vec<f32> = order.windows(2).map(|p| p[1].0.y - p[0].1.y).collect();
        assert!((gaps[0] - gaps[1]).abs() < 1e-5, "{gaps:?}");
        assert!(distribute(&boxes()[..2], true).iter().all(|m| *m == Vec2::ZERO), "two have nothing between them");
    }
}
