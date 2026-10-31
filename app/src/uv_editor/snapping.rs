//! Snapping: what a drag on the layout lands on when it comes close.
//!
//! Targets are lines and points of the layout - the grid, guides, the unit
//! square's edges and middle, each island's box and its vertices and the
//! middles of its edges, and the other labels' edges and middles - gathered
//! once when a drag begins. Anything that drags asks the same [`Targets`]:
//! a move snaps the edges and middle of what is moving, a handle or the
//! measure tool snaps the one point under the pointer, a guide snaps its
//! line.
//!
//! The distance is in screen pixels, turned into layout units by the view's
//! scale, so it feels the same at any zoom.

use tequila_core::math::{vec2, Vec2};

/// How close, in screen pixels, a drag has to come to snap.
pub const PIXELS: f32 = 8.0;

/// What was snapped to, which says so beside the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    // Lowest first: of two targets as near, the later in this list wins.
    Grid,
    LayoutEdge,
    LayoutCentre,
    IslandEdge,
    IslandCentre,
    Vertex,
    Midpoint,
    LabelEdge,
    LabelCentre,
    Guide,
}

impl Kind {
    /// The note shown beside the pointer.
    pub fn note(self) -> &'static str {
        match self {
            Kind::Grid => "grid",
            Kind::LayoutEdge => "layout edge",
            Kind::LayoutCentre => "layout centre",
            Kind::IslandEdge => "island edge",
            Kind::IslandCentre => "island centre",
            Kind::Vertex => "vertex",
            Kind::Midpoint => "edge midpoint",
            Kind::LabelEdge => "label edge",
            Kind::LabelCentre => "label centre",
            Kind::Guide => "guide",
        }
    }
}

/// A line to snap to: across the layout at `at` along one axis, over
/// `from..to` along the other (where the highlight is drawn).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub at: f32,
    pub from: f32,
    pub to: f32,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub at: Vec2,
    pub kind: Kind,
}

/// A grid to snap to: lines every `step` along each axis from the origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub step: Vec2,
}

/// Everything a drag can snap to.
#[derive(Clone, Debug, Default)]
pub struct Targets {
    /// Lines down the layout, at a place along `u`.
    pub across: Vec<Line>,
    /// Lines across the layout, at a place along `v`.
    pub down: Vec<Line>,
    pub points: Vec<Point>,
    pub grid: Option<Grid>,
}

/// One axis snapped: to the line at `at`, which the drag's `anchor` landed
/// on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisHit {
    pub line: Line,
    pub anchor: f32,
}

/// What a drag snapped to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snap {
    /// How far to move what is dragged to land on it.
    pub delta: Vec2,
    /// A line down the layout snapped to, across.
    pub across: Option<AxisHit>,
    /// A line across the layout snapped to, down.
    pub down: Option<AxisHit>,
    /// A point snapped to, both ways at once.
    pub point: Option<Point>,
}

impl Snap {
    pub fn none() -> Snap {
        Snap::default()
    }

    pub fn is_some(&self) -> bool {
        self.across.is_some() || self.down.is_some() || self.point.is_some()
    }

    /// What it snapped to, for the note beside the pointer and the status
    /// bar: the point, or the more particular of the lines.
    pub fn kind(&self) -> Option<Kind> {
        if let Some(point) = self.point {
            return Some(point.kind);
        }
        match (self.across, self.down) {
            (Some(a), Some(d)) => Some(a.line.kind.max(d.line.kind)),
            (Some(a), None) => Some(a.line.kind),
            (None, Some(d)) => Some(d.line.kind),
            (None, None) => None,
        }
    }

    /// The note: "grid point" where a grid line snapped each way.
    pub fn note(&self) -> Option<&'static str> {
        match (self.point, self.across, self.down) {
            (None, Some(a), Some(d)) if a.line.kind == Kind::Grid && d.line.kind == Kind::Grid => Some("grid point"),
            _ => self.kind().map(Kind::note),
        }
    }
}

/// Of two candidates, the nearer; of two as near, the more particular.
fn better(distance: f32, kind: Kind, best: Option<(f32, Kind)>) -> bool {
    match best {
        None => true,
        Some((d, k)) => distance < d - 1e-7 || ((distance - d).abs() <= 1e-7 && kind > k),
    }
}

impl Targets {
    pub fn new() -> Targets {
        Targets::default()
    }

    /// A line down the layout at `u`, over `from..to` of `v`.
    pub fn line_across(&mut self, u: f32, from: f32, to: f32, kind: Kind) {
        self.across.push(Line { at: u, from: from.min(to), to: from.max(to), kind });
    }

    /// A line across the layout at `v`, over `from..to` of `u`.
    pub fn line_down(&mut self, v: f32, from: f32, to: f32, kind: Kind) {
        self.down.push(Line { at: v, from: from.min(to), to: from.max(to), kind });
    }

    pub fn point(&mut self, at: Vec2, kind: Kind) {
        self.points.push(Point { at, kind });
    }

    /// A box square to the layout: its four edges and its middle.
    pub fn rectangle(&mut self, low: Vec2, high: Vec2, edge: Kind, centre: Kind) {
        self.line_across(low.x, low.y, high.y, edge);
        self.line_across(high.x, low.y, high.y, edge);
        self.line_down(low.y, low.x, high.x, edge);
        self.line_down(high.y, low.x, high.x, edge);
        let middle = (low + high) * 0.5;
        self.point(middle, centre);
        // Its middle lines too, so an edge can be put on an island's middle.
        self.line_across(middle.x, low.y, high.y, centre);
        self.line_down(middle.y, low.x, high.x, centre);
    }

    /// The nearest line of `lines`, or of the grid along that axis, to any
    /// of `anchors`, within `tolerance`.
    fn nearest_line(lines: &[Line], grid: Option<f32>, anchors: &[f32], tolerance: f32) -> Option<AxisHit> {
        let mut best: Option<(f32, Kind)> = None;
        let mut hit = None;
        for &anchor in anchors {
            for line in lines {
                let distance = (line.at - anchor).abs();
                if distance <= tolerance && better(distance, line.kind, best) {
                    best = Some((distance, line.kind));
                    hit = Some(AxisHit { line: *line, anchor });
                }
            }
            if let Some(step) = grid.filter(|s| *s > 0.0) {
                let at = (anchor / step).round() * step;
                let distance = (at - anchor).abs();
                if distance <= tolerance && better(distance, Kind::Grid, best) {
                    best = Some((distance, Kind::Grid));
                    hit = Some(AxisHit { line: Line { at, from: f32::NEG_INFINITY, to: f32::INFINITY, kind: Kind::Grid }, anchor });
                }
            }
        }
        hit
    }

    /// The nearest point target to any of `anchors`, within `tolerance`, and
    /// which anchor it was.
    fn nearest_point(&self, anchors: &[Vec2], tolerance: f32) -> Option<(Point, Vec2, f32)> {
        let mut best: Option<(f32, Kind)> = None;
        let mut hit = None;
        for &anchor in anchors {
            for point in &self.points {
                let distance = (point.at - anchor).length();
                if distance <= tolerance && better(distance, point.kind, best) {
                    best = Some((distance, point.kind));
                    hit = Some((*point, anchor, distance));
                }
            }
        }
        hit
    }

    /// Snap one point - a handle, an end of a measure - to whatever is near.
    pub fn snap_point(&self, at: Vec2, tolerance: f32) -> Snap {
        self.snap_shape(&[at.x], &[at.y], &[at], tolerance)
    }

    /// Snap a shape being moved, by its anchors: the places along `u` its
    /// edges and middle are at (`across`), the same along `v` (`down`), and
    /// the points that may land on a point target (its middle). Each axis
    /// snaps on its own; a point target near enough takes both.
    pub fn snap_shape(&self, across: &[f32], down: &[f32], points: &[Vec2], tolerance: f32) -> Snap {
        let grid = self.grid;
        let mut snap = Snap::none();
        snap.across = Targets::nearest_line(&self.across, grid.map(|g| g.step.x), across, tolerance);
        snap.down = Targets::nearest_line(&self.down, grid.map(|g| g.step.y), down, tolerance);
        let lines = vec2(
            snap.across.map(|h| (h.line.at - h.anchor).abs()).unwrap_or(f32::INFINITY),
            snap.down.map(|h| (h.line.at - h.anchor).abs()).unwrap_or(f32::INFINITY),
        );
        // A point wins over lines no more particular than it, or nearer
        // than any of them: a label's middle on an island's middle is one
        // thing aimed at, not two lines that happen to cross there.
        if let Some((point, anchor, distance)) = self.nearest_point(points, tolerance) {
            let take = match snap.kind() {
                None => true,
                Some(kind) => point.kind >= kind || distance <= lines.x.min(lines.y),
            };
            if take {
                return Snap { delta: point.at - anchor, across: None, down: None, point: Some(point) };
            }
        }
        snap.delta = vec2(
            snap.across.map(|h| h.line.at - h.anchor).unwrap_or(0.0),
            snap.down.map(|h| h.line.at - h.anchor).unwrap_or(0.0),
        );
        snap
    }

    /// Snap a guide: a line down the layout (`across`) or across it, to the
    /// lines and the grid along the same axis.
    pub fn snap_line(&self, across: bool, at: f32, tolerance: f32) -> Snap {
        let grid = self.grid;
        let mut snap = Snap::none();
        if across {
            snap.across = Targets::nearest_line(&self.across, grid.map(|g| g.step.x), &[at], tolerance);
            snap.delta.x = snap.across.map(|h| h.line.at - at).unwrap_or(0.0);
        } else {
            snap.down = Targets::nearest_line(&self.down, grid.map(|g| g.step.y), &[at], tolerance);
            snap.delta.y = snap.down.map(|h| h.line.at - at).unwrap_or(0.0);
        }
        snap
    }
}

/// A turn snapped to the nearest multiple of `step`, both in radians.
pub fn snap_angle(angle: f32, step: f32) -> f32 {
    (angle / step).round() * step
}

/// The end of a line from `from` constrained to the nearest of the eight
/// directions every 45 degrees, at the length it has.
pub fn constrained(from: Vec2, to: Vec2) -> Vec2 {
    let d = to - from;
    let length = d.length();
    if length <= 0.0 {
        return to;
    }
    let angle = snap_angle(d.y.atan2(d.x), std::f32::consts::FRAC_PI_4);
    from + vec2(angle.cos(), angle.sin()) * length
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-5
    }

    fn layout_square() -> Targets {
        let mut targets = Targets::new();
        targets.rectangle(Vec2::ZERO, Vec2::splat(1.0), Kind::LayoutEdge, Kind::LayoutCentre);
        targets
    }

    /// A point within the distance lands on the nearest line each way; one
    /// further off is left where it is.
    #[test]
    fn a_point_lands_on_the_nearest_lines() {
        let mut targets = layout_square();
        targets.line_across(0.3, 0.0, 1.0, Kind::Guide);
        let snap = targets.snap_point(vec2(0.305, 0.996), 0.01);
        assert!(near(snap.delta, vec2(-0.005, 0.004)), "{:?}", snap.delta);
        assert_eq!(snap.across.unwrap().line.kind, Kind::Guide);
        assert_eq!(snap.down.unwrap().line.kind, Kind::LayoutEdge);
        assert_eq!(snap.note(), Some("guide"), "the more particular of the two says what it snapped to");
        let free = targets.snap_point(vec2(0.35, 0.6), 0.01);
        assert!(!free.is_some() && free.delta == Vec2::ZERO);
    }

    /// A point target near enough takes both axes at once, and says so.
    #[test]
    fn a_point_target_takes_both_axes() {
        let mut targets = layout_square();
        targets.rectangle(vec2(0.2, 0.2), vec2(0.4, 0.6), Kind::IslandEdge, Kind::IslandCentre);
        let snap = targets.snap_point(vec2(0.304, 0.397), 0.01);
        assert_eq!(snap.point.map(|p| p.kind), Some(Kind::IslandCentre));
        assert!(near(snap.delta, vec2(-0.004, 0.003)));
        assert_eq!(snap.note(), Some("island centre"));
    }

    /// The grid snaps at every step, its lines and where they cross.
    #[test]
    fn the_grid_snaps_its_lines_and_points() {
        let targets = Targets { grid: Some(Grid { step: vec2(0.125, 0.25) }), ..Targets::new() };
        let snap = targets.snap_point(vec2(0.377, 0.74), 0.02);
        assert!(near(snap.delta, vec2(0.375 - 0.377, 0.75 - 0.74)), "{:?}", snap.delta);
        assert_eq!(snap.note(), Some("grid point"));
        let one_way = targets.snap_point(vec2(0.31, 0.745), 0.02);
        assert_eq!(one_way.across, None);
        assert_eq!(one_way.note(), Some("grid"));
        // Of a grid line and a guide as near, the guide.
        let mut both = targets.clone();
        both.line_down(0.75, 0.0, 1.0, Kind::Guide);
        assert_eq!(both.snap_point(vec2(0.31, 0.745), 0.02).down.unwrap().line.kind, Kind::Guide);
    }

    /// A moving shape snaps by whichever of its edges or middle comes
    /// nearest: its right edge onto a guide moves the whole of it.
    #[test]
    fn a_moving_shape_snaps_by_its_nearest_edge() {
        let mut targets = Targets::new();
        targets.line_across(0.5, 0.0, 1.0, Kind::Guide);
        targets.line_down(0.2, 0.0, 1.0, Kind::LabelEdge);
        // A box from 0.1 to 0.494 across, 0.31 to 0.41 down.
        let (low, high) = (vec2(0.1, 0.31), vec2(0.494, 0.41));
        let middle = (low + high) * 0.5;
        let snap = targets.snap_shape(&[low.x, middle.x, high.x], &[low.y, middle.y, high.y], &[middle], 0.012);
        assert!(near(snap.delta, vec2(0.006, 0.0)), "{:?}", snap.delta);
        assert_eq!(snap.across.unwrap().anchor, high.x);
        assert_eq!(snap.down, None, "the label edge at 0.2 is too far from the top at 0.31");
    }

    /// A guide snaps along its own axis only.
    #[test]
    fn a_guide_snaps_to_lines_along_its_axis() {
        let mut targets = layout_square();
        targets.line_down(0.42, 0.0, 1.0, Kind::IslandEdge);
        let down = targets.snap_line(false, 0.425, 0.01);
        assert!(near(down.delta, vec2(0.0, -0.005)));
        assert_eq!(down.note(), Some("island edge"));
        let across = targets.snap_line(true, 0.425, 0.01);
        assert!(!across.is_some(), "a line along the other axis is not one to land on");
    }

    #[test]
    fn turns_and_measures_keep_to_their_steps() {
        let fifteen = 15f32.to_radians();
        assert!((snap_angle(37f32.to_radians(), fifteen) - 30f32.to_radians()).abs() < 1e-5);
        assert!((snap_angle(-8f32.to_radians(), fifteen) + fifteen).abs() < 1e-5);
        let end = constrained(vec2(0.0, 0.0), vec2(1.0, 0.1));
        assert!(near(end, vec2((1.01f32).sqrt(), 0.0)), "{end:?}");
        let diagonal = constrained(vec2(0.0, 0.0), vec2(1.0, 0.9));
        assert!((diagonal.x - diagonal.y).abs() < 1e-5);
    }
}
