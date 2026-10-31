//! A part's layout as the editor draws it and snaps to it: every edge once,
//! which ones are seams, the islands they bound, and how large the part is
//! on the layout - its size, where that is the same all over, being what
//! lets the rulers read in millimetres.

use std::collections::HashMap;
use tequila_core::math::{vec2, Vec2};

/// How far apart the densest and the sparsest tenth of a layout may be, as
/// a ratio of their scale, and the layout still have one size: an unwrap's
/// pieces bend a little as they flatten, and an artist's layout packed at
/// whatever scale fitted does not.
pub const EVEN: f32 = 1.5;

pub struct Layout {
    pub triangles: Vec<[Vec2; 3]>,
    pub island_of: Vec<u32>,
    pub edges: Vec<[Vec2; 2]>,
    pub seams: Vec<[Vec2; 2]>,
    /// Each island's corners, a point for every corner of every triangle.
    pub points: Vec<Vec<Vec2>>,
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
}

impl Layout {
    pub fn of(mesh: &tequila_scene::Mesh, set: usize) -> Option<Layout> {
        let uv = mesh.uv_sets.get(set)?;
        if uv.len() != mesh.positions.len() || mesh.triangle_count() == 0 {
            return None;
        }
        let count = mesh.triangle_count();
        // Islands: triangles joined by the vertices they share; an unwrap
        // splits its vertices along every seam.
        let mut parent: Vec<u32> = (0..mesh.positions.len() as u32).collect();
        fn find(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                parent[x as usize] = parent[parent[x as usize] as usize];
                x = parent[x as usize];
            }
            x
        }
        for t in 0..count {
            let [a, b, c] = mesh.triangle(t);
            for (p, q) in [(a, b), (b, c)] {
                let (rp, rq) = (find(&mut parent, p), find(&mut parent, q));
                if rp != rq {
                    parent[rp as usize] = rq;
                }
            }
        }
        let mut ids: HashMap<u32, u32> = HashMap::new();
        let mut island_of = Vec::with_capacity(count);
        let mut triangles = Vec::with_capacity(count);
        let mut uses: HashMap<(u32, u32), u32> = HashMap::new();
        let (mut surface, mut flat) = (0.0f64, 0.0f64);
        // Each triangle's scale, its square root of surface over layout
        // area, weighted by its surface: how even the layout is.
        let mut scales: Vec<(f32, f32)> = Vec::with_capacity(count);
        for t in 0..count {
            let [a, b, c] = mesh.triangle(t);
            let root = find(&mut parent, a);
            let next = ids.len() as u32;
            island_of.push(*ids.entry(root).or_insert(next));
            let corners = [uv[a as usize], uv[b as usize], uv[c as usize]];
            triangles.push(corners);
            for (p, q) in [(a, b), (b, c), (c, a)] {
                *uses.entry((p.min(q), p.max(q))).or_default() += 1;
            }
            let (pa, pb, pc) = (mesh.positions[a as usize], mesh.positions[b as usize], mesh.positions[c as usize]);
            let area = (pb - pa).cross(pc - pa).length() * 0.5;
            surface += area as f64;
            let [ua, ub, uc] = corners;
            let laid = (((ub.x - ua.x) * (uc.y - ua.y) - (ub.y - ua.y) * (uc.x - ua.x)) * 0.5).abs();
            flat += laid as f64;
            if laid > 1e-12 && area > 0.0 {
                scales.push(((area / laid).sqrt(), area));
            }
        }
        let mut edges = Vec::new();
        let mut seams = Vec::new();
        let mut midpoints = Vec::with_capacity(uses.len());
        let mut lengths = Vec::with_capacity(uses.len());
        for (&(p, q), &n) in &uses {
            let edge = [uv[p as usize], uv[q as usize]];
            midpoints.push((edge[0] + edge[1]) * 0.5);
            lengths.push((edge[1] - edge[0]).length());
            if n == 1 {
                seams.push(edge);
            } else {
                edges.push(edge);
            }
        }
        let mut points: Vec<Vec<Vec2>> = vec![Vec::new(); ids.len()];
        for (t, corners) in triangles.iter().enumerate() {
            points[island_of[t] as usize].extend_from_slice(corners);
        }
        let axes = points.iter().map(|p| principal_axis(p)).collect();
        let boxes = points
            .iter()
            .map(|p| {
                let low = p.iter().fold(Vec2::splat(f32::INFINITY), |a, q| a.min(*q));
                let high = p.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, q| a.max(*q));
                (low, high)
            })
            .collect();
        // The vertices once: an unwrap gives each corner of a seam a vertex
        // of its own on each side, at one place on each.
        let mut seen = std::collections::HashSet::new();
        let mut vertices = Vec::new();
        for p in uv {
            if seen.insert((p.x.to_bits(), p.y.to_bits())) {
                vertices.push(*p);
            }
        }
        lengths.sort_by(|a, b| a.total_cmp(b));
        let typical_edge = lengths.get(lengths.len() / 2).copied().unwrap_or(0.0);
        let units_per_uv = if flat > 0.0 { (surface / flat).sqrt() as f32 } else { 0.0 };
        Some(Layout {
            triangles,
            island_of,
            edges,
            seams,
            points,
            axes,
            boxes,
            vertices,
            midpoints,
            typical_edge,
            units_per_uv,
            even: units_per_uv > 0.0 && evenness(&mut scales) <= EVEN,
        })
    }

    /// The island under a point of the layout.
    pub fn at(&self, uv: Vec2) -> Option<usize> {
        self.triangles.iter().position(|t| inside(uv, t)).map(|t| self.island_of[t] as usize)
    }

    /// Scene units per layout unit, where the layout has one size.
    pub fn density(&self) -> Option<f32> {
        self.even.then_some(self.units_per_uv)
    }

    /// The box holding every island.
    pub fn extent(&self) -> Option<(Vec2, Vec2)> {
        self.boxes.iter().copied().reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
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

fn principal_axis(points: &[Vec2]) -> f32 {
    if points.is_empty() {
        return 0.0;
    }
    let n = points.len() as f32;
    let mean = points.iter().fold(Vec2::ZERO, |a, p| a + *p) / n;
    let (mut xx, mut xy, mut yy) = (0.0f32, 0.0f32, 0.0f32);
    for p in points {
        let d = *p - mean;
        xx += d.x * d.x;
        xy += d.x * d.y;
        yy += d.y * d.y;
    }
    0.5 * (2.0 * xy).atan2(xx - yy)
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
}
