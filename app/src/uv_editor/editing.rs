//! The layout's own items - islands, edges, vertices - as the editor picks
//! them and moves them: what a click and a box pick, what a selection moves
//! and names in a command, and the map a drag makes, all worked out here on
//! their own and tested so.
//!
//! A drag of the layout is previewed, not sent: it is a map of the layout
//! (`UvAffine`) the canvas draws the selection through while the button is
//! held, and one `uv.transform` when it is let go, as the engine wants a
//! whole move rather than its steps.

use super::layout::{self, Layout};
use super::selection::{Item, Mode, Selection};
use super::snapping::{self, Snap, Targets};
use super::tools::Frame;
use std::collections::HashSet;
use tequila_core::math::{vec2, Vec2};
use tequila_scene::uv::{Distortion, EdgeKind, UvAffine, UvSelection};

/// What is under the pointer, of the layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hover {
    pub triangle: Option<u32>,
    pub island: Option<u32>,
    /// An edge within reach, and a vertex.
    pub edge: Option<u32>,
    pub vertex: Option<u32>,
}

impl Hover {
    pub fn at(layout: &Layout, at: Vec2, reach: f32) -> Hover {
        let hit = layout.uv.triangle_at(at);
        Hover {
            triangle: hit.map(|h| h.triangle),
            island: hit.map(|h| h.island),
            edge: layout.uv.nearest_edge(at, reach).map(|(e, _)| e),
            vertex: layout.uv.nearest_vertex(at, reach).map(|(v, _)| v),
        }
    }
}

/// An island as a selection names it: by its first triangle.
pub fn island_item(layout: &Layout, island: u32) -> Option<Item> {
    layout.uv.islands.get(island as usize).and_then(|i| i.triangles.first()).map(|&t| Item::Island(t))
}

/// What a click at `at` picks in `mode`: the island under it, or the edge
/// or vertex within `reach`.
pub fn pick(layout: &Layout, mode: Mode, at: Vec2, reach: f32) -> Option<Item> {
    match mode {
        Mode::Labels => None,
        Mode::Islands => layout.uv.island_at(at).and_then(|i| island_item(layout, i)),
        Mode::Edges => layout.uv.nearest_edge(at, reach).map(|(e, _)| Item::Edge(layout.uv.edges[e as usize].vertices)),
        Mode::Vertices => layout.uv.nearest_vertex(at, reach).map(|(v, _)| Item::Vertex(layout.uv.uv_vertices[v as usize].vertex)),
    }
}

/// What a box from `from` to `to` picks in `mode`: the islands it reaches
/// into, or holds the middle of; the edges and vertices inside it.
pub fn boxed(layout: &Layout, mode: Mode, from: Vec2, to: Vec2) -> Vec<Item> {
    let area = layout::rect(from, to);
    let inside = |p: Vec2| layout::contains(area, p);
    let middle = (area.0 + area.1) * 0.5;
    match mode {
        Mode::Labels => Vec::new(),
        Mode::Islands => {
            let under = layout.uv.island_at(middle);
            (0..layout.uv.islands.len() as u32)
                .filter(|&i| {
                    let island = &layout.uv.islands[i as usize];
                    layout::overlaps(area, (island.min, island.max)) && (under == Some(i) || island.uv_vertices.iter().any(|&v| inside(layout.uv.uv_vertices[v as usize].uv)))
                })
                .filter_map(|i| island_item(layout, i))
                .collect()
        }
        Mode::Edges => (0..layout.uv.edges.len() as u32).filter(|&e| layout.ends(e).iter().all(|p| inside(*p))).map(|e| Item::Edge(layout.uv.edges[e as usize].vertices)).collect(),
        Mode::Vertices => layout.uv.uv_vertices.iter().filter(|v| inside(v.uv)).map(|v| Item::Vertex(v.vertex)).collect(),
    }
}

/// Everything of the mode's kind.
pub fn all(layout: &Layout, mode: Mode) -> Vec<Item> {
    match mode {
        Mode::Labels => Vec::new(),
        Mode::Islands => (0..layout.uv.islands.len() as u32).filter_map(|i| island_item(layout, i)).collect(),
        Mode::Edges => layout.uv.edges.iter().map(|e| Item::Edge(e.vertices)).collect(),
        Mode::Vertices => layout.uv.uv_vertices.iter().map(|v| Item::Vertex(v.vertex)).collect(),
    }
}

/// The islands selected, by their numbers now, each once, in order.
pub fn islands(layout: &Layout, selection: &Selection) -> Vec<u32> {
    let mut out: Vec<u32> = selection.island_triangles().iter().filter_map(|&t| layout.uv.island_of.get(t as usize).copied()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The islands the selection is on, whatever it holds: the islands
/// themselves, or those its edges and vertices are on. What an operation on
/// islands - a pack, a straighten - takes from an edge or a vertex picked.
pub fn islands_touched(layout: &Layout, selection: &Selection) -> Vec<u32> {
    let mut out = islands(layout, selection);
    for edge in selection.edges() {
        if let Some(e) = layout.edge_named(edge) {
            out.push(layout.uv.edges[e as usize].island);
        }
    }
    for vertex in selection.vertices() {
        if let Some(i) = layout.uv.island_of_vertex(vertex) {
            out.push(i);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The mesh vertices the selection's edges and vertices are on.
pub fn mesh_vertices(selection: &Selection) -> Vec<u32> {
    let mut out: Vec<u32> = selection.vertices();
    for [a, b] in selection.edges() {
        out.push(a);
        out.push(b);
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// What a move of the selection names in `uv.transform`: whole islands, or
/// the vertices its edges and vertices are on.
pub fn uv_selection(layout: &Layout, selection: &Selection) -> Option<UvSelection> {
    let out = if selection.island_triangles().is_empty() { UvSelection::Vertices(mesh_vertices(selection)) } else { UvSelection::Islands(islands(layout, selection)) };
    (!out.is_empty()).then_some(out)
}

/// The UV vertices a move of the selection carries, sorted.
pub fn moving(layout: &Layout, selection: &Selection) -> Vec<u32> {
    match uv_selection(layout, selection) {
        Some(UvSelection::Islands(list)) => {
            let mut out: Vec<u32> = list.iter().flat_map(|&i| layout.uv.islands[i as usize].uv_vertices.iter().copied()).collect();
            out.sort_unstable();
            out
        }
        Some(UvSelection::Vertices(list)) => layout.uv.uv_vertices_of(&list),
        None => Vec::new(),
    }
}

/// The box square to the layout round some of its UV vertices, as a frame
/// that is not turned: what a layout selection's handles are on.
pub fn frame_of(layout: &Layout, vertices: &[u32], through: &UvAffine) -> Option<Frame> {
    let (mut low, mut high) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for &v in vertices {
        let p = through.apply(layout.uv.uv_vertices[v as usize].uv);
        low = low.min(p);
        high = high.max(p);
    }
    (low.x <= high.x).then(|| Frame { centre: (low + high) * 0.5, size: high - low, angle: 0.0 })
}

/// The seams a stitch joins: the selected edges that are seams, or the
/// seams between the selected islands, or inside one of them.
pub fn stitch_edges(layout: &Layout, selection: &Selection) -> Vec<[u32; 2]> {
    let islands = islands(layout, selection);
    if !islands.is_empty() {
        return layout.uv.shared_seams(&islands);
    }
    selection.edges().into_iter().filter(|&edge| layout.edge_named(edge).is_some_and(|e| layout.uv.edges[e as usize].kind == EdgeKind::Seam)).collect()
}

/// The selected edges that are not seams already, which a cut would part.
pub fn cuttable(layout: &Layout, selection: &Selection) -> Vec<[u32; 2]> {
    selection.edges().into_iter().filter(|&edge| layout.edge_named(edge).is_some_and(|e| layout.uv.edges[e as usize].kind == EdgeKind::Inner)).collect()
}

/// The selection renamed after an edit: an island by its first triangle
/// in the new layout, and what is no longer there let go.
pub fn renamed(layout: &Layout, item: Item, vertices: usize) -> Option<Item> {
    match item {
        Item::Island(t) => layout.uv.island_of.get(t as usize).and_then(|&i| island_item(layout, i)),
        Item::Edge(edge) => layout.edge_named(edge).map(|_| item),
        Item::Vertex(v) => ((v as usize) < vertices).then_some(item),
        other => Some(other),
    }
}

// ---- the map a drag makes ----------------------------------------------------

/// Moved by `delta`.
pub fn moved(delta: Vec2) -> UvAffine {
    UvAffine::translation(delta)
}

/// Turned by `angle` about `pivot`: a positive turn takes u toward v, which
/// the editor, drawing v down, shows clockwise.
pub fn turned(pivot: Vec2, angle: f32) -> UvAffine {
    UvAffine::rotation(angle).about(pivot)
}

/// Scaled by `k` across and down about `pivot`.
pub fn scaled(pivot: Vec2, k: Vec2) -> UvAffine {
    UvAffine::scale(k).about(pivot)
}

/// The Rotate tool, or the turn handle: turned about the frame's middle by
/// how far the pointer has gone round it, in steps of `step` with Shift.
pub fn tool_turn(frame: &Frame, from: Vec2, to: Vec2, step: Option<f32>) -> (UvAffine, f32) {
    let pivot = frame.centre;
    let angle = |p: Vec2| (p.y - pivot.y).atan2(p.x - pivot.x);
    let mut turn = angle(to) - angle(from);
    if let Some(step) = step {
        turn = snapping::snap_angle(turn, step);
    }
    (turned(pivot, turn), turn)
}

/// The Scale tool: scaled about the frame's middle by how far the pointer
/// is from it against where it was pressed; with `apart`, across and down
/// each by its own.
pub fn tool_scale(frame: &Frame, from: Vec2, to: Vec2, apart: bool) -> UvAffine {
    let pivot = frame.centre;
    let (a, b) = (from - pivot, to - pivot);
    let k = if apart {
        vec2(if a.x.abs() > 1e-6 { (b.x / a.x).abs() } else { 1.0 }, if a.y.abs() > 1e-6 { (b.y / a.y).abs() } else { 1.0 })
    } else {
        Vec2::splat(b.length() / a.length().max(1e-6))
    };
    scaled(pivot, k.max(Vec2::splat(0.02)))
}

/// A handle of the frame dragged to `pointer`: a corner (`across` and
/// `down` each -1 or 1) or the middle of an edge (one of them 0), scaling
/// with the opposite side held, or about the middle with `about_middle`. A
/// corner keeps the shape unless `free`.
pub fn handle_scale(frame: &Frame, across: i8, down: i8, pointer: Vec2, free: bool, about_middle: bool) -> UvAffine {
    let (sx, sy) = (across as f32, down as f32);
    let anchor = if about_middle { frame.centre } else { frame.handle(-sx, -sy) };
    let reach = frame.handle(sx, sy) - anchor;
    let now = pointer - anchor;
    let mut k = vec2(if across != 0 && reach.x.abs() > 1e-9 { now.x / reach.x } else { 1.0 }, if down != 0 && reach.y.abs() > 1e-9 { now.y / reach.y } else { 1.0 });
    if across != 0 && down != 0 && !free {
        let along = (now.x * reach.x + now.y * reach.y) / (reach.x * reach.x + reach.y * reach.y).max(1e-12);
        k = Vec2::splat(along);
    }
    scaled(anchor, k.max(Vec2::splat(0.02)))
}

/// A move of a box, snapped by its edges and middle to `targets` within
/// `tolerance`; with `straight`, kept to across or down, whichever it is
/// more. How far it moves, and what it snapped to.
pub fn snapped_move(targets: &Targets, (low, high): (Vec2, Vec2), mut delta: Vec2, straight: bool, snap: bool, tolerance: f32) -> (Vec2, Snap) {
    if straight {
        if delta.x.abs() >= delta.y.abs() {
            delta.y = 0.0;
        } else {
            delta.x = 0.0;
        }
    }
    if !snap {
        return (delta, Snap::none());
    }
    let (low, high) = (low + delta, high + delta);
    let middle = (low + high) * 0.5;
    let mut hit = targets.snap_shape(&[low.x, middle.x, high.x], &[low.y, middle.y, high.y], &[middle], tolerance);
    if straight {
        if delta.y == 0.0 {
            hit.delta.y = 0.0;
            hit.down = None;
        } else {
            hit.delta.x = 0.0;
            hit.across = None;
        }
        if hit.point.is_some() {
            hit = Snap::none();
        }
    }
    (delta + hit.delta, hit)
}

/// The change of a frame typed into the Transform panel, from `from` to
/// `to` about the panel's `pivot`: scaled and turned about it, then moved.
pub fn panel_change(from: &Frame, to: &Frame, pivot: Vec2, moved_to: Vec2) -> UvAffine {
    let k = vec2(to.size.x / from.size.x.max(1e-9), to.size.y / from.size.y.max(1e-9));
    let mut map = UvAffine::IDENTITY;
    if (k - Vec2::ONE).length() > 1e-6 {
        map = map.then(&scaled(pivot, k));
    }
    let turn = to.angle - from.angle;
    if turn.abs() > 1e-7 {
        map = map.then(&turned(pivot, turn));
    }
    let shift = moved_to - pivot;
    if shift.length() > 0.0 {
        map = map.then(&moved(shift));
    }
    map
}

// ---- what a selection measures --------------------------------------------------

/// The area of some islands on the layout and on the part (in the part's
/// square units), and their stretch: area and angle, each averaged by
/// surface, 1 where the layout is true.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measures {
    pub uv_area: f32,
    pub surface: f32,
    pub area: f32,
    pub angle: f32,
}

pub fn measure(layout: &Layout, islands: &[u32], distortion: Option<&[Distortion]>) -> Measures {
    let mut out = Measures::default();
    let (mut weight, mut area, mut angle) = (0.0f64, 0.0f64, 0.0f64);
    for &i in islands {
        let Some(island) = layout.uv.islands.get(i as usize) else { continue };
        out.uv_area += island.uv_area;
        out.surface += island.surface_area;
        if let Some(all) = distortion {
            for &t in &island.triangles {
                let (Some(d), Some(&w)) = (all.get(t as usize), layout.surface.get(t as usize)) else { continue };
                if d.area.is_finite() && d.angle.is_finite() {
                    weight += w as f64;
                    area += (d.area * w) as f64;
                    angle += (d.angle * w) as f64;
                }
            }
        }
    }
    if weight > 0.0 {
        out.area = (area / weight) as f32;
        out.angle = (angle / weight) as f32;
    } else {
        out.area = 1.0;
        out.angle = 1.0;
    }
    out
}

// ---- labels carried with their islands ---------------------------------------

/// The likeness of one set of places to another - a turn, a scale, a move,
/// and a mirror if that fits better - found by least squares: how an island
/// went from where it was to where an edit put it. Mapping `p` gives
/// `to_centre + scale * R(angle) * F(p - from_centre)`, F mirroring across
/// first when `mirror`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Likeness {
    pub from_centre: Vec2,
    pub to_centre: Vec2,
    pub angle: f32,
    pub scale: f32,
    pub mirror: bool,
}

impl Likeness {
    pub fn apply(&self, p: Vec2) -> Vec2 {
        let mut d = p - self.from_centre;
        if self.mirror {
            d.x = -d.x;
        }
        let (s, c) = self.angle.sin_cos();
        self.to_centre + vec2(d.x * c - d.y * s, d.x * s + d.y * c) * self.scale
    }

    /// Whether it changes nothing worth carrying a label for.
    pub fn is_still(&self) -> bool {
        !self.mirror && self.angle.abs() < 1e-6 && (self.scale - 1.0).abs() < 1e-6 && (self.to_centre - self.from_centre).length() < 1e-7
    }
}

/// The likeness taking `before` to `after`, point for point (Umeyama's
/// method, in two dimensions). None for fewer than two points apart.
pub fn likeness(before: &[Vec2], after: &[Vec2]) -> Option<Likeness> {
    let n = before.len().min(after.len());
    if n < 2 {
        return None;
    }
    let mean = |p: &[Vec2]| p[..n].iter().fold(Vec2::ZERO, |a, q| a + *q) / n as f32;
    let (from_centre, to_centre) = (mean(before), mean(after));
    let (mut spread, mut dot, mut cross, mut dot_m, mut cross_m) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (p, q) in before[..n].iter().zip(&after[..n]) {
        let (p, q) = (*p - from_centre, *q - to_centre);
        spread += (p.x * p.x + p.y * p.y) as f64;
        dot += (p.x * q.x + p.y * q.y) as f64;
        cross += (p.x * q.y - p.y * q.x) as f64;
        // Mirrored across first: p becomes (-x, y).
        dot_m += (-p.x * q.x + p.y * q.y) as f64;
        cross_m += (-p.x * q.y - p.y * q.x) as f64;
    }
    if spread <= 1e-24 {
        return None;
    }
    let (plain, mirrored) = ((dot * dot + cross * cross).sqrt(), (dot_m * dot_m + cross_m * cross_m).sqrt());
    let mirror = mirrored > plain * (1.0 + 1e-6);
    let (d, c, strength) = if mirror { (dot_m, cross_m, mirrored) } else { (dot, cross, plain) };
    Some(Likeness { from_centre, to_centre, angle: c.atan2(d) as f32, scale: (strength / spread) as f32, mirror })
}

/// The edges selected, as a set to look up by layout edge.
pub fn selected_edges(layout: &Layout, selection: &Selection) -> HashSet<u32> {
    selection.edges().into_iter().filter_map(|e| layout.edge_named(e)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-4
    }

    /// Two squares of a part side by side, each its own island.
    fn two_squares() -> Layout {
        let positions = vec![
            vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0),
            vec3(2.0, 0.0, 0.0), vec3(3.0, 0.0, 0.0), vec3(3.0, 1.0, 0.0), vec3(2.0, 1.0, 0.0),
        ];
        let mut mesh = tequila_scene::Mesh::new(positions, vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
        let square = |x: f32| [vec2(x, 0.1), vec2(x + 0.3, 0.1), vec2(x + 0.3, 0.4), vec2(x, 0.4)];
        mesh.uv_sets = vec![square(0.1).into_iter().chain(square(0.6)).collect()];
        Layout::of(&mesh, 0).unwrap()
    }

    /// A click picks the island under it, the edge or the vertex near it;
    /// a box the islands it reaches, the edges and vertices inside it.
    #[test]
    fn clicks_and_boxes_pick_the_layout_s_items() {
        let layout = two_squares();
        assert_eq!(pick(&layout, Mode::Islands, vec2(0.75, 0.25), 0.01), Some(Item::Island(2)), "the second island, by its first triangle");
        assert_eq!(pick(&layout, Mode::Islands, vec2(0.5, 0.25), 0.01), None);
        let Some(Item::Edge([a, b])) = pick(&layout, Mode::Edges, vec2(0.25, 0.102), 0.01) else { panic!("an edge") };
        assert_eq!([a.min(b), a.max(b)], [0, 1], "the first square's top edge");
        assert_eq!(pick(&layout, Mode::Vertices, vec2(0.905, 0.405), 0.01), Some(Item::Vertex(6)));
        assert_eq!(boxed(&layout, Mode::Islands, vec2(0.35, 0.0), vec2(0.65, 0.2)).len(), 2, "a box over both reaches both");
        assert_eq!(boxed(&layout, Mode::Islands, vec2(0.2, 0.2), vec2(0.3, 0.3)), vec![Item::Island(0)], "a box inside an island picks it");
        assert_eq!(boxed(&layout, Mode::Vertices, vec2(0.0, 0.0), vec2(0.45, 0.45)).len(), 4);
        assert_eq!(boxed(&layout, Mode::Edges, vec2(0.0, 0.0), vec2(0.45, 0.45)).len(), 5, "four sides and the diagonal");
        assert_eq!(all(&layout, Mode::Islands).len(), 2);
    }

    /// A selection names in a command what it moves: islands by number, or
    /// the vertices its edges and vertices are on.
    #[test]
    fn a_selection_names_what_it_moves() {
        let layout = two_squares();
        let mut selection = Selection::default();
        selection.set(Item::Island(2));
        assert_eq!(uv_selection(&layout, &selection), Some(UvSelection::Islands(vec![1])));
        assert_eq!(moving(&layout, &selection).len(), 4);
        let frame = frame_of(&layout, &moving(&layout, &selection), &UvAffine::IDENTITY).unwrap();
        assert!(near(frame.centre, vec2(0.75, 0.25)) && near(frame.size, vec2(0.3, 0.3)));
        selection.set_all([Item::Edge([1, 0])]);
        assert_eq!(uv_selection(&layout, &selection), Some(UvSelection::Vertices(vec![0, 1])), "an edge moves the vertices at its ends");
        assert_eq!(cuttable(&layout, &selection), Vec::<[u32; 2]>::new(), "the edge of the part is no edge to cut");
        selection.set_all([Item::Vertex(5), Item::Vertex(0)]);
        assert_eq!(islands_touched(&layout, &selection), vec![0, 1]);
        assert!(stitch_edges(&layout, &selection).is_empty(), "no seams between two squares apart");
    }

    /// The maps a drag makes: a turn about the frame's middle, in steps with
    /// Shift; a scale about it; a corner dragged out with the far corner
    /// held, keeping the shape; an edge's middle stretching one way.
    #[test]
    fn a_drag_makes_the_map_it_shows() {
        let frame = Frame { centre: vec2(0.5, 0.5), size: vec2(0.2, 0.2), angle: 0.0 };
        let (turn, angle) = tool_turn(&frame, vec2(0.7, 0.5), vec2(0.5, 0.71), Some(15f32.to_radians()));
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5, "{angle}");
        assert!(near(turn.apply(vec2(0.6, 0.5)), vec2(0.5, 0.6)), "a quarter turn clockwise on screen takes u toward v");
        let double = tool_scale(&frame, vec2(0.6, 0.5), vec2(0.7, 0.5), false);
        assert!(near(double.apply(vec2(0.6, 0.6)), vec2(0.7, 0.7)));
        let corner = handle_scale(&frame, 1, 1, vec2(0.8, 0.79), false, false);
        assert!(near(corner.apply(vec2(0.4, 0.4)), vec2(0.4, 0.4)), "the far corner stays");
        assert!(near(corner.apply(vec2(0.6, 0.6)), vec2(0.795, 0.795)), "the shape kept: {:?}", corner.apply(vec2(0.6, 0.6)));
        let side = handle_scale(&frame, 1, 0, vec2(0.7, 0.9), false, true);
        assert!(near(side.apply(vec2(0.6, 0.6)), vec2(0.7, 0.6)), "about the middle, across only");
        let typed = panel_change(&frame, &Frame { size: vec2(0.4, 0.2), ..frame }, vec2(0.4, 0.4), vec2(0.4, 0.4));
        assert!(near(typed.apply(vec2(0.6, 0.6)), vec2(0.8, 0.6)), "wider about the top left");
    }

    /// A move snaps by the box's nearest edge, and kept straight it loses
    /// what it snapped to the other way.
    #[test]
    fn a_layout_move_snaps_by_its_box() {
        let mut targets = Targets::new();
        targets.line_across(0.5, 0.0, 1.0, snapping::Kind::Guide);
        targets.line_down(0.3, 0.0, 1.0, snapping::Kind::IslandEdge);
        let (delta, snap) = snapped_move(&targets, (vec2(0.1, 0.1), vec2(0.3, 0.2)), vec2(0.195, 0.004), false, true, 0.01);
        assert!(near(delta, vec2(0.2, 0.004)), "the right edge on the guide, the top too far from the island edge to snap: {delta:?}");
        assert_eq!(snap.note(), Some("guide"));
        let (straight, _) = snapped_move(&targets, (vec2(0.1, 0.1), vec2(0.3, 0.2)), vec2(0.195, 0.096), true, true, 0.01);
        assert!(near(straight, vec2(0.2, 0.0)), "across only: {straight:?}");
        let (free, none) = snapped_move(&targets, (vec2(0.1, 0.1), vec2(0.3, 0.2)), vec2(0.195, 0.004), false, false, 0.01);
        assert!(near(free, vec2(0.195, 0.004)) && !none.is_some());
    }

    /// An island's move, turn and scale found again from its places before
    /// and after, a mirror too: how a label on it is carried with it.
    #[test]
    fn an_island_s_likeness_is_found_from_its_places() {
        let before = [vec2(0.1, 0.1), vec2(0.4, 0.1), vec2(0.4, 0.2), vec2(0.1, 0.2)];
        let map = turned(vec2(0.25, 0.15), 0.6).then(&scaled(vec2(0.0, 0.0), vec2(2.0, 2.0))).then(&moved(vec2(0.3, -0.05)));
        let after: Vec<Vec2> = before.iter().map(|p| map.apply(*p)).collect();
        let found = likeness(&before, &after).unwrap();
        assert!(!found.mirror && (found.angle - 0.6).abs() < 1e-4 && (found.scale - 2.0).abs() < 1e-4, "{found:?}");
        let label = vec2(0.2, 0.12);
        assert!(near(found.apply(label), map.apply(label)));
        let flip = scaled(vec2(0.25, 0.15), vec2(-1.0, 1.0));
        let flipped: Vec<Vec2> = before.iter().map(|p| flip.apply(*p)).collect();
        let found = likeness(&before, &flipped).unwrap();
        assert!(found.mirror && (found.scale - 1.0).abs() < 1e-4, "{found:?}");
        assert!(near(found.apply(label), flip.apply(label)));
        assert!(likeness(&before, &before).unwrap().is_still());
    }
}
