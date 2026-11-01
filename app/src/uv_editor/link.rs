//! The editor and the viewport, each pointing at the other: what is under
//! the pointer on the layout - an island, a face, an edge, a vertex - is
//! lit on the part in the viewport, and the face under the viewport's
//! pointer, on the part the editor shows, is lit on the layout. Faces
//! picked in the viewport at Face level are drawn on both, for the Islands
//! panel to select their islands or cut them out.
//!
//! The viewport's highlight is drawn over its picture, not into it: the
//! part's triangles that face the camera, projected; those another part
//! hides, found by a ray each when what is lit or the view changes, lit
//! faintly, as seen through it.

use crate::project::Project;
use tequila_core::math::{Vec2, Vec3};
use tequila_scene::{NodeId, NodeKind};

/// What the viewport is to light on the editor's part.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hint {
    pub node: Option<NodeId>,
    /// Faces of the part to fill: the island or the face under the pointer.
    pub triangles: Vec<u32>,
    /// Edges to draw, by the mesh vertices at their ends: the island's
    /// outline, or the edge under the pointer and its seam's other side.
    pub edges: Vec<[u32; 2]>,
    /// Vertices to mark: the vertex under the pointer, the pins.
    pub vertices: Vec<u32>,
}

/// The face under the viewport's pointer, on the part the editor shows:
/// its number and where on it, as weights on its corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportHover {
    pub node: NodeId,
    pub triangle: u32,
    pub weights: [f32; 3],
}

/// The weights on a triangle's corners of a point on it.
pub fn weights(p: Vec3, [a, b, c]: [Vec3; 3]) -> [f32; 3] {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
    let d = d00 * d11 - d01 * d01;
    if d.abs() <= f32::MIN_POSITIVE {
        return [1.0, 0.0, 0.0];
    }
    let v = (d11 * d20 - d01 * d21) / d;
    let w = (d00 * d21 - d01 * d20) / d;
    [1.0 - v - w, v, w]
}

/// The face a viewport hit is on, as the editor wants it.
pub fn hover_of(project: &Project, node: NodeId, triangle: u32, at: Vec3) -> Option<ViewportHover> {
    let document = project.document();
    let NodeKind::Part { mesh } = document.tree.get(node)?.kind else { return None };
    let mesh = document.geometry.get(mesh)?;
    if triangle as usize >= mesh.triangle_count() {
        return None;
    }
    let world = document.tree.world_transform(node);
    let corners = mesh.triangle(triangle as usize).map(|v| world.point(mesh.positions[v as usize]));
    Some(ViewportHover { node, triangle, weights: weights(at, corners) })
}

/// A point of the layout from weights on a triangle's corners there.
pub fn on_layout(corners: [Vec2; 3], w: [f32; 3]) -> Vec2 {
    corners[0] * w[0] + corners[1] * w[1] + corners[2] * w[2]
}

/// Which of the hint's faces the camera sees, each by a ray from the eye to
/// its middle that meets nothing before it: what is behind another part is
/// lit faintly, as seen through it. `pick` is the viewport's.
pub fn seen(project: &Project, camera: &tequila_render_core::camera::Camera, hint: &Hint, pick: impl Fn(&tequila_core::math::Ray) -> Option<(Option<NodeId>, u32, f32)>) -> Vec<bool> {
    let Some(node) = hint.node else { return Vec::new() };
    let document = project.document();
    let Some(NodeKind::Part { mesh }) = document.tree.get(node).map(|n| &n.kind) else { return Vec::new() };
    let Some(mesh) = document.geometry.get(*mesh) else { return Vec::new() };
    let world = document.tree.world_transform(node);
    let eye = camera.position();
    hint.triangles
        .iter()
        .map(|&t| {
            if t as usize >= mesh.triangle_count() {
                return true;
            }
            let [a, b, c] = mesh.triangle(t as usize).map(|v| world.point(mesh.positions[v as usize]));
            let middle = (a + b + c) / 3.0;
            let to = middle - eye;
            let distance = to.length();
            if distance <= 0.0 {
                return true;
            }
            match pick(&tequila_core::math::Ray::new(eye, to / distance)) {
                None => true,
                Some((hit_node, primitive, t_hit)) => (hit_node == Some(node) && primitive == t) || t_hit >= distance * 0.995,
            }
        })
        .collect()
}

/// Light the hint on the part in the viewport, and the faces picked there
/// at Face level on it. `seen` says, face by face, which the camera sees.
pub fn draw(ui: &egui::Ui, rect: egui::Rect, camera: &tequila_render_core::camera::Camera, project: &Project, hint: &Hint, seen: &[bool]) {
    let Some(node) = hint.node else { return };
    let document = project.document();
    let Some(NodeKind::Part { mesh }) = document.tree.get(node).map(|n| &n.kind) else { return };
    let Some(mesh) = document.geometry.get(*mesh) else { return };
    if !document.tree.is_effectively_visible(node) {
        return;
    }
    let world = document.tree.world_transform(node);
    let eye = camera.position();
    let painter = ui.painter_at(rect);
    let at = |v: u32| mesh.positions.get(v as usize).map(|p| world.point(*p));
    let screen = |p: Vec3| crate::gizmo_ui::project_point(camera, rect, p);
    let fill = |triangles: &[u32], colour: egui::Color32, seen: &[bool]| {
        let mut shape = egui::Mesh::default();
        for (i, &t) in triangles.iter().enumerate() {
            if t as usize >= mesh.triangle_count() {
                continue;
            }
            // Behind another part: faint, as seen through it.
            let colour = if seen.get(i).copied().unwrap_or(true) { colour } else { colour.gamma_multiply(0.3) };
            let [a, b, c] = mesh.triangle(t as usize).map(|v| world.point(mesh.positions[v as usize]));
            // Only what faces the camera: the back of a part through its
            // front reads as a second island.
            if (b - a).cross(c - a).dot(eye - a) <= 0.0 {
                continue;
            }
            let (Some(pa), Some(pb), Some(pc)) = (screen(a), screen(b), screen(c)) else { continue };
            let base = shape.vertices.len() as u32;
            for p in [pa, pb, pc] {
                shape.colored_vertex(p, colour);
            }
            shape.add_triangle(base, base + 1, base + 2);
        }
        painter.add(egui::Shape::mesh(shape));
    };
    // The faces picked at Face level, then what the editor points at.
    let picked: Vec<u32> = project.selection.faces(node).collect();
    if !picked.is_empty() {
        fill(&picked, crate::theme::hue(crate::theme::Hue::Blue).gamma_multiply(0.45), &[]);
    }
    let colour = crate::theme::hue(crate::theme::Hue::Green);
    fill(&hint.triangles, colour.gamma_multiply(0.42), seen);
    for [a, b] in &hint.edges {
        if let (Some(a), Some(b)) = (at(*a).and_then(screen), at(*b).and_then(screen)) {
            painter.line_segment([a, b], egui::Stroke::new(2.5_f32, colour));
        }
    }
    for v in &hint.vertices {
        if let Some(p) = at(*v).and_then(screen) {
            painter.circle_filled(p, 4.5, colour);
            painter.circle_stroke(p, 4.5, egui::Stroke::new(1.0_f32, egui::Color32::WHITE));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};

    #[test]
    fn a_point_on_a_face_lands_where_it_is_on_the_layout() {
        let corners = [vec3(0.0, 0.0, 0.0), vec3(2.0, 0.0, 0.0), vec3(0.0, 2.0, 0.0)];
        let w = weights(vec3(0.5, 1.0, 0.0), corners);
        assert!((w[0] - 0.25).abs() < 1e-5 && (w[1] - 0.25).abs() < 1e-5 && (w[2] - 0.5).abs() < 1e-5, "{w:?}");
        let at = on_layout([vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0)], w);
        assert!((at - vec2(0.25, 0.5)).length() < 1e-5);
    }
}
