//! A part's layout as the editor draws it and snaps to it: the layout
//! engine's island model (`tequila_scene::uv::UvLayout`), which every edit
//! of the layout reads too, so the islands, seams and vertices picked here
//! are the ones the commands name; and what the canvas wants of it each
//! frame, worked out once - each triangle's corners, the islands' boxes,
//! every vertex once - and how large the part is on the layout: its size,
//! where that is the same all over, being what lets the rulers read in
//! millimetres.

use std::collections::HashMap;
use tequila_core::math::{vec2, Vec2};
use tequila_scene::uv::{EdgeKind, UvLayout};

/// How far apart the densest and the sparsest tenth of a layout may be, as
/// a ratio of their scale, and the layout still have one size: an unwrap's
/// pieces bend a little as they flatten, and an artist's layout packed at
/// whatever scale fitted does not.
pub const EVEN: f32 = 1.5;

pub struct Layout {
    /// The engine's islands, edges and vertices, and its hit tests.
    pub uv: UvLayout,
    /// Each triangle's corners on the layout.
    pub triangles: Vec<[Vec2; 3]>,
    /// Each island's vertices on the layout: what a label is fitted inside.
    pub points: Vec<Vec<Vec2>>,
    /// The direction each island runs, as the engine straightens it.
    pub axes: Vec<f32>,
    /// Each island's box, square to the layout.
    pub boxes: Vec<(Vec2, Vec2)>,
    /// Every vertex of the layout once, for snapping.
    pub vertices: Vec<Vec2>,
    /// The middle of every edge, for snapping.
    pub midpoints: Vec<Vec2>,
    /// The length of a middling edge, in layout units: how far in the view
    /// has to be for vertices to be told apart.
    pub typical_edge: f32,
    /// Scene units of the part per unit of the layout, averaged over it.
    pub units_per_uv: f32,
    /// Whether the layout has that one size all over (`EVEN`).
    pub even: bool,
    /// Each mesh vertex's UV vertex, where a triangle uses it.
    pub vertex_uv: Vec<u32>,
    /// Each triangle's surface on the part, for the stretch of a selection.
    pub surface: Vec<f32>,
    /// The layout edge each pair of mesh vertices names on its side, the
    /// smaller first.
    edge_of: HashMap<(u32, u32), u32>,
}

impl Layout {
    pub fn of(mesh: &tequila_scene::Mesh, set: usize) -> Option<Layout> {
        if mesh.triangle_count() == 0 {
            return None;
        }
        let uv = UvLayout::new(mesh, set)?;
        let count = mesh.triangle_count();
        let triangles: Vec<[Vec2; 3]> = (0..count as u32).map(|t| uv.triangle_uv(t)).collect();
        let (mut surface_total, mut flat) = (0.0f64, 0.0f64);
        // Each triangle's scale, its square root of surface over layout
        // area, weighted by its surface: how even the layout is.
        let mut scales: Vec<(f32, f32)> = Vec::with_capacity(count);
        let mut surface = Vec::with_capacity(count);
        for (t, [ua, ub, uc]) in triangles.iter().enumerate() {
            let [a, b, c] = mesh.triangle(t);
            let (pa, pb, pc) = (mesh.positions[a as usize], mesh.positions[b as usize], mesh.positions[c as usize]);
            let area = (pb - pa).cross(pc - pa).length() * 0.5;
            surface.push(area);
            surface_total += area as f64;
            let laid = (((ub.x - ua.x) * (uc.y - ua.y) - (ub.y - ua.y) * (uc.x - ua.x)) * 0.5).abs();
            flat += laid as f64;
            if laid > 1e-12 && area > 0.0 {
                scales.push(((area / laid).sqrt(), area));
            }
        }
        let place = |v: u32| uv.uv_vertices[v as usize].uv;
        let mut midpoints = Vec::with_capacity(uv.edges.len());
        let mut lengths = Vec::with_capacity(uv.edges.len());
        let mut edge_of = HashMap::with_capacity(uv.edges.len());
        for (e, edge) in uv.edges.iter().enumerate() {
            let [a, b] = edge.ends.map(place);
            midpoints.push((a + b) * 0.5);
            lengths.push((b - a).length());
            let [p, q] = edge.vertices;
            edge_of.entry((p.min(q), p.max(q))).or_insert(e as u32);
        }
        let points: Vec<Vec<Vec2>> = uv.islands.iter().map(|i| i.uv_vertices.iter().map(|&v| place(v)).collect()).collect();
        let axes = uv.islands.iter().map(|i| i.straightening_angle()).collect();
        let boxes = uv.islands.iter().map(|i| (i.min, i.max)).collect();
        // The vertices once: a seam's two sides are two UV vertices each,
        // and where they lie on one place they are one to snap to.
        let mut seen = std::collections::HashSet::new();
        let mut vertices = Vec::new();
        for v in &uv.uv_vertices {
            if seen.insert((v.uv.x.to_bits(), v.uv.y.to_bits())) {
                vertices.push(v.uv);
            }
        }
        let mut vertex_uv = vec![u32::MAX; mesh.positions.len()];
        for (c, &v) in mesh.indices.iter().enumerate() {
            let slot = &mut vertex_uv[v as usize];
            if *slot == u32::MAX {
                *slot = uv.corner_vertex[c];
            }
        }
        lengths.sort_by(|a, b| a.total_cmp(b));
        let typical_edge = lengths.get(lengths.len() / 2).copied().unwrap_or(0.0);
        let units_per_uv = if flat > 0.0 { (surface_total / flat).sqrt() as f32 } else { 0.0 };
        let even = units_per_uv > 0.0 && evenness(&mut scales) <= EVEN;
        Some(Layout { uv, triangles, points, axes, boxes, vertices, midpoints, typical_edge, units_per_uv, even, vertex_uv, surface, edge_of })
    }

    /// The island under a point of the layout.
    pub fn at(&self, uv: Vec2) -> Option<usize> {
        self.uv.island_at(uv).map(|i| i as usize)
    }

    /// Scene units per layout unit, where the layout has one size.
    pub fn density(&self) -> Option<f32> {
        self.even.then_some(self.units_per_uv)
    }

    /// The box holding every island.
    pub fn extent(&self) -> Option<(Vec2, Vec2)> {
        self.boxes.iter().copied().reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    }

    /// An edge's two ends on the layout.
    pub fn ends(&self, edge: u32) -> [Vec2; 2] {
        self.uv.edges[edge as usize].ends.map(|v| self.uv.uv_vertices[v as usize].uv)
    }

    /// The layout edge a pair of mesh vertices names, either way round: on
    /// the side those vertices are on, so a seam picked on one island is
    /// that side of it.
    pub fn edge_named(&self, [a, b]: [u32; 2]) -> Option<u32> {
        self.edge_of.get(&(a.min(b), a.max(b))).copied()
    }

    /// Where a mesh vertex is on the layout.
    pub fn place_of(&self, vertex: u32) -> Option<Vec2> {
        let v = *self.vertex_uv.get(vertex as usize)?;
        self.uv.uv_vertices.get(v as usize).map(|v| v.uv)
    }

    /// How many seams there are, each counted once.
    pub fn seam_count(&self) -> usize {
        self.uv.edges.iter().filter(|e| e.kind == EdgeKind::Seam).count() / 2
    }
}

/// How far apart the scale of the densest tenth of a layout is from the
/// sparsest tenth's, by surface: 1 for a layout of one size all over.
pub fn evenness(scales: &mut [(f32, f32)]) -> f32 {
    if scales.is_empty() {
        return f32::INFINITY;
    }
    scales.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f32 = scales.iter().map(|(_, w)| w).sum();
    let at = |share: f32| {
        let mut sum = 0.0;
        for (scale, weight) in scales.iter() {
            sum += weight;
            if sum >= share * total {
                return *scale;
            }
        }
        scales.last().map(|s| s.0).unwrap_or(0.0)
    };
    let (low, high) = (at(0.1), at(0.9));
    if low <= 0.0 {
        f32::INFINITY
    } else {
        high / low
    }
}

pub fn inside(p: Vec2, t: &[Vec2; 3]) -> bool {
    let side = |a: Vec2, b: Vec2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let (d1, d2, d3) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
    let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(negative && positive)
}

/// Whether a point is inside a quad given corner by corner round.
pub fn inside_quad(p: Vec2, quad: &[Vec2; 4]) -> bool {
    inside(p, &[quad[0], quad[1], quad[2]]) || inside(p, &[quad[0], quad[2], quad[3]])
}

/// A rectangle square to the layout, from two corners in any order.
pub fn rect(a: Vec2, b: Vec2) -> (Vec2, Vec2) {
    (a.min(b), a.max(b))
}

/// Whether two boxes square to the layout overlap.
pub fn overlaps(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> bool {
    a.0.x <= b.1.x && b.0.x <= a.1.x && a.0.y <= b.1.y && b.0.y <= a.1.y
}

/// Whether a point is inside a box square to the layout.
pub fn contains(area: (Vec2, Vec2), p: Vec2) -> bool {
    p.x >= area.0.x && p.y >= area.0.y && p.x <= area.1.x && p.y <= area.1.y
}

#[allow(dead_code)]
pub fn centre_of(b: (Vec2, Vec2)) -> Vec2 {
    vec2((b.0.x + b.1.x) * 0.5, (b.0.y + b.1.y) * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    /// Two squares of a part a metre across, laid out at the same scale and
    /// then one at twice another's: the first has one size, the second not.
    #[test]
    fn a_layout_of_one_size_reads_in_millimetres() {
        let positions = vec![
            vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0),
            vec3(2.0, 0.0, 0.0), vec3(3.0, 0.0, 0.0), vec3(3.0, 1.0, 0.0), vec3(2.0, 1.0, 0.0),
        ];
        let indices = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
        let mut mesh = tequila_scene::Mesh::new(positions.clone(), indices.clone());
        let square = |x: f32, side: f32| [vec2(x, 0.0), vec2(x + side, 0.0), vec2(x + side, side), vec2(x, side)];
        mesh.uv_sets = vec![square(0.0, 0.4).into_iter().chain(square(0.5, 0.4)).collect()];
        let layout = Layout::of(&mesh, 0).expect("a layout");
        assert_eq!(layout.points.len(), 2, "two islands");
        assert!(layout.even);
        assert!((layout.density().unwrap() - 2.5).abs() < 1e-4, "a metre is 0.4 of the layout: {:?}", layout.density());
        assert!((layout.boxes[1].0 - vec2(0.5, 0.0)).length() < 1e-6);
        assert_eq!(layout.vertices.len(), 8);
        assert_eq!(layout.at(vec2(0.6, 0.1)), Some(1));

        let mut uneven = tequila_scene::Mesh::new(positions, indices);
        uneven.uv_sets = vec![square(0.0, 0.4).into_iter().chain(square(0.5, 0.1)).collect()];
        let layout = Layout::of(&uneven, 0).unwrap();
        assert!(!layout.even && layout.density().is_none(), "one island four times the other's scale");
    }

    /// The engine's edges, found again by the mesh vertices that name them,
    /// and each mesh vertex's place.
    #[test]
    fn edges_and_vertices_are_found_by_their_mesh_numbers() {
        let mut mesh = tequila_scene::Mesh::new(vec![vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)], vec![0, 1, 2, 0, 2, 3]);
        mesh.uv_sets = vec![vec![vec2(0.0, 1.0), vec2(1.0, 1.0), vec2(1.0, 0.0), vec2(0.0, 0.0)]];
        let layout = Layout::of(&mesh, 0).unwrap();
        let diagonal = layout.edge_named([2, 0]).expect("the diagonal");
        assert_eq!(layout.uv.edges[diagonal as usize].kind, EdgeKind::Inner);
        let ends = layout.ends(diagonal);
        assert!(ends.contains(&vec2(0.0, 1.0)) && ends.contains(&vec2(1.0, 0.0)));
        assert_eq!(layout.place_of(3), Some(vec2(0.0, 0.0)));
        assert_eq!(layout.edge_named([1, 3]), None, "no edge between opposite corners");
        assert_eq!(layout.seam_count(), 0);
    }
}
