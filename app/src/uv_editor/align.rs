//! Align and distribute: labels and islands lined up by their edges,
//! middles or corners, against each other, an island, the layout's square or
//! the 2D cursor, and spread so the gaps between them are equal. Worked on
//! the boxes square to the layout that hold each, as turned; what comes back
//! is how far to move each. The layout's vertices and edge paths line up in
//! the engine (`tequila_scene::uv::vertices`), against the same targets.

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
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Align {
    pub const ALL: [Align; 6] = [Align::Left, Align::Centre, Align::Right, Align::Top, Align::Middle, Align::Bottom];
    pub const CORNERS: [Align; 4] = [Align::TopLeft, Align::TopRight, Align::BottomLeft, Align::BottomRight];

    /// Which way it lines up across and down: -1 the low side (left, top),
    /// 0 the middle, 1 the high side; none where it leaves that way alone.
    pub fn sides(self) -> (Option<i8>, Option<i8>) {
        match self {
            Align::Left => (Some(-1), None),
            Align::Centre => (Some(0), None),
            Align::Right => (Some(1), None),
            Align::Top => (None, Some(-1)),
            Align::Middle => (None, Some(0)),
            Align::Bottom => (None, Some(1)),
            Align::TopLeft => (Some(-1), Some(-1)),
            Align::TopRight => (Some(1), Some(-1)),
            Align::BottomLeft => (Some(-1), Some(1)),
            Align::BottomRight => (Some(1), Some(1)),
        }
    }

    pub fn icon(self) -> crate::icons::Icon {
        use crate::icons::Icon;
        match self {
            Align::Left => Icon::AlignLeft,
            Align::Centre => Icon::AlignCentre,
            Align::Right => Icon::AlignRight,
            Align::Top => Icon::AlignTop,
            Align::Middle => Icon::AlignMiddle,
            Align::Bottom => Icon::AlignBottom,
            Align::TopLeft => Icon::AlignTopLeft,
            Align::TopRight => Icon::AlignTopRight,
            Align::BottomLeft => Icon::AlignBottomLeft,
            Align::BottomRight => Icon::AlignBottomRight,
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
            Align::TopLeft => "Align top left corners",
            Align::TopRight => "Align top right corners",
            Align::BottomLeft => "Align bottom left corners",
            Align::BottomRight => "Align bottom right corners",
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
            Align::TopLeft => "top_left",
            Align::TopRight => "top_right",
            Align::BottomLeft => "bottom_left",
            Align::BottomRight => "bottom_right",
        }
    }

    pub fn from_key(key: &str) -> Option<Align> {
        match key {
            "center" => Some(Align::Centre),
            _ => Align::ALL.into_iter().chain(Align::CORNERS).find(|a| a.key() == key),
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
    /// The 2D cursor: a point, so left, middle and right edges all go to it.
    Cursor,
}

impl Target {
    pub const ALL: [Target; 4] = [Target::Selection, Target::Island, Target::Layout, Target::Cursor];

    pub fn label(self) -> &'static str {
        match self {
            Target::Selection => "Selection",
            Target::Island => "Island",
            Target::Layout => "Layout",
            Target::Cursor => "2D cursor",
        }
    }

    /// Its name on the scripting channel.
    pub fn key(self) -> &'static str {
        match self {
            Target::Selection => "selection",
            Target::Island => "island",
            Target::Layout => "layout",
            Target::Cursor => "cursor",
        }
    }

    pub fn from_key(key: &str) -> Option<Target> {
        Target::ALL.into_iter().find(|t| t.key() == key)
    }
}

/// The box holding all of `boxes`.
pub fn union(boxes: &[Box2]) -> Option<Box2> {
    boxes.iter().copied().reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
}

/// How far to move each box to line it up with `target` as `how` says: a
/// corner by both its sides. A target that is a point - the 2D cursor - has
/// its edges and middle all there.
pub fn align(boxes: &[Box2], target: Box2, how: Align) -> Vec<Vec2> {
    let (across, down) = how.sides();
    let to = |side: i8, low: f32, high: f32, t_low: f32, t_high: f32| match side {
        s if s < 0 => t_low - low,
        0 => (t_low + t_high) * 0.5 - (low + high) * 0.5,
        _ => t_high - high,
    };
    boxes
        .iter()
        .map(|&(low, high)| {
            let x = across.map_or(0.0, |side| to(side, low.x, high.x, target.0.x, target.1.x));
            let y = down.map_or(0.0, |side| to(side, low.y, high.y, target.0.y, target.1.y));
            vec2(x, y)
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
        // A corner both ways; to a point, its corner onto the point.
        let corner = align(&boxes(), all, Align::BottomRight);
        assert!(near(corner[0], vec2(0.7, 0.5)) && near(corner[1], vec2(0.0, 0.55)), "{corner:?}");
        let cursor = (vec2(0.5, 0.5), vec2(0.5, 0.5));
        assert!(near(align(&boxes()[..1], cursor, Align::TopLeft)[0], vec2(0.4, 0.4)));
        assert!(near(align(&boxes()[..1], cursor, Align::Centre)[0], vec2(0.35, 0.0)));
        for how in Align::ALL.into_iter().chain(Align::CORNERS) {
            assert_eq!(Align::from_key(how.key()), Some(how));
        }
        for target in Target::ALL {
            assert_eq!(Target::from_key(target.key()), Some(target));
        }
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
