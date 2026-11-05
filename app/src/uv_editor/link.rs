//! The editor and the viewport, each pointing at the other: what is under
//! the pointer on the layout - an island, a face, an edge, a vertex - is
//! lit on the part in the viewport, with the layout's seams, what is
//! selected and the pins; and the face under the viewport's pointer, on the
//! part the editor shows, is lit on the layout. Faces picked in the
//! viewport at Face level are drawn on both, for the Islands panel to
//! select their islands or cut them out.
//!
//! The viewport's highlight is drawn over its picture, not into it, so it
//! is the same over every shading - the graphics card's, the processor's
//! Solid, the path tracer's. What of it the camera cannot see is found by
//! rays through the same geometry the viewport picks against, when what is
//! lit or the view changes ([`sight`]): the part's triangles that face the
//! camera are filled, those another part hides faintly; a line is cut into
//! pieces a few points long and each piece, and each vertex, is shown
//! where nothing is in front of it, faint where another part is, and not
//! at all where the part's own surface is - the back of a part does not
//! show through its front.

use crate::project::Project;
use std::collections::HashMap;
use tequila_core::math::{Ray, Vec2, Vec3};
use tequila_scene::{NodeId, NodeKind};

/// What the viewport is to light on the editor's part.
#[derive(Clone, Debug, Default, PartialEq, Hash)]
pub struct Hint {
    pub node: Option<NodeId>,
    /// Faces of the part to fill: the island or the face under the pointer.
    pub triangles: Vec<u32>,
    /// Edges to draw, by the mesh vertices at their ends: the island's
    /// outline, or the edge under the pointer and its seam's other side.
    pub edges: Vec<[u32; 2]>,
    /// Vertices to mark: the vertex under the pointer.
    pub vertices: Vec<u32>,
    /// The layout's seams, in its layout modes.
    pub seams: Vec<[u32; 2]>,
    /// What is selected on the layout: the selected islands' outlines and
    /// the selected edges, and the selected vertices.
    pub selected_edges: Vec<[u32; 2]>,
    pub selected_vertices: Vec<u32>,
    /// The pins on the set shown.
    pub pins: Vec<u32>,
}

impl Hint {
    /// Every line it draws, each once.
    fn lines(&self) -> Vec<[u32; 2]> {
        let mut out: Vec<[u32; 2]> = self.edges.iter().chain(&self.seams).chain(&self.selected_edges).map(|&[a, b]| [a.min(b), a.max(b)]).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Every vertex it marks, each once.
    fn points(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self.vertices.iter().chain(&self.selected_vertices).chain(&self.pins).copied().collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// What the sight of it was worked out for: the hint, the part's
    /// geometry as built, the camera and the viewport's size.
    pub fn sight_key(&self, rebuilds: u64, camera: &tequila_render_core::camera::Camera, rect: egui::Rect) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        rebuilds.hash(&mut hasher);
        format!("{camera:?}").hash(&mut hasher);
        [rect.width().to_bits(), rect.height().to_bits()].hash(&mut hasher);
        hasher.finish()
    }
}

/// How much of something the camera sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    /// Nothing in front of it.
    Seen,
    /// Another part in front of it: drawn faintly, as seen through that.
    Behind,
    /// The part's own surface in front of it: not drawn.
    Hidden,
}

/// What the camera sees of a hint: for each face to fill whether it is
/// seen; for each line, piece by piece; for each vertex.
#[derive(Clone, Debug, Default)]
pub struct Sight {
    pub faces: Vec<bool>,
    pub lines: HashMap<[u32; 2], Vec<Seen>>,
    pub points: HashMap<u32, Seen>,
}

impl Sight {
    fn line(&self, [a, b]: [u32; 2]) -> &[Seen] {
        self.lines.get(&[a.min(b), a.max(b)]).map(Vec::as_slice).unwrap_or(&[Seen::Seen])
    }
}

/// How long a piece of a line is on screen, in points, at most.
const PIECE: f32 = 8.0;
/// How many pieces a line is cut into at most.
const MOST_PIECES: usize = 16;
/// How many rays a hint's lines may take: past it every line is one piece,
/// so a part with thousands of seams still orbits smoothly.
const RAY_BUDGET: usize = 24_000;

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

/// Where the camera looks at a point of the scene from: the ray through the
/// point, as the viewport's picking makes it, and how far along it the
/// point is. None for a point behind the camera.
fn ray_to(camera: &tequila_render_core::camera::Camera, aspect: f32, p: Vec3) -> Option<(Ray, f32)> {
    let film = camera.project(p, aspect)?;
    let ray = camera.pick_ray(film, aspect);
    let length = ray.direction.length();
    if length <= 0.0 {
        return None;
    }
    let ray = Ray::new(ray.origin, ray.direction / length);
    Some((ray, (p - ray.origin).dot(ray.direction)))
}

/// What the camera sees of the hint on its part: the faces by a ray to the
/// middle of each that meets nothing before it; the lines and vertices by
/// a ray to each piece's middle and each vertex.
///
/// A line whose faces on the part all face away from the camera is on the
/// part's far side, and hidden whatever a ray says: its rays graze the
/// part's outline and are no test of it. Otherwise a ray that meets one of
/// the line's own faces, or the vertex's, just short of the point still
/// sees it. `pick` is the viewport's: what a ray meets first, its part, its
/// face and how far.
pub fn sight(project: &Project, camera: &tequila_render_core::camera::Camera, rect: egui::Rect, hint: &Hint, pick: impl Fn(&Ray) -> Option<(Option<NodeId>, u32, f32)>) -> Sight {
    let mut out = Sight::default();
    let Some(node) = hint.node else { return out };
    let document = project.document();
    let Some(NodeKind::Part { mesh }) = document.tree.get(node).map(|n| &n.kind) else { return out };
    let Some(mesh) = document.geometry.get(*mesh) else { return out };
    let world = document.tree.world_transform(node);
    let aspect = rect.width().max(1.0) / rect.height().max(1.0);
    let at = |v: u32| mesh.positions.get(v as usize).map(|p| world.point(*p));
    let is_face = |t: u32| (t as usize) < mesh.triangle_count();
    out.faces = hint
        .triangles
        .iter()
        .map(|&t| {
            if !is_face(t) {
                return true;
            }
            let [a, b, c] = mesh.triangle(t as usize).map(|v| world.point(mesh.positions[v as usize]));
            let Some((ray, distance)) = ray_to(camera, aspect, (a + b + c) / 3.0) else { return true };
            match pick(&ray) {
                None => true,
                Some((hit_node, primitive, t_hit)) => (hit_node == Some(node) && primitive == t) || t_hit >= distance * 0.995,
            }
        })
        .collect();

    // The faces round each vertex the hint marks or a line of it ends at,
    // found by place, so a vertex split along a seam or a crease has the
    // faces on both sides.
    let lines = hint.lines();
    let points = hint.points();
    let key = |v: u32| mesh.positions.get(v as usize).map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]);
    let mut faces_at: HashMap<[u32; 3], Vec<u32>> = lines.iter().flatten().chain(&points).filter_map(|&v| key(v)).map(|k| (k, Vec::new())).collect();
    for t in 0..mesh.triangle_count() {
        for v in mesh.triangle(t) {
            if let Some(list) = key(v).and_then(|k| faces_at.get_mut(&k)) {
                if list.last() != Some(&(t as u32)) {
                    list.push(t as u32);
                }
            }
        }
    }
    let around = |v: u32| key(v).and_then(|k| faces_at.get(&k)).map(Vec::as_slice).unwrap_or(&[]);
    // Whether a face of the part faces the camera.
    let orthographic = matches!(camera.projection, tequila_render_core::camera::Projection::Orthographic { .. });
    let (eye, forward) = (camera.position(), camera.forward());
    let facing = |t: u32| {
        let [a, b, c] = mesh.triangle(t as usize).map(|v| world.point(mesh.positions[v as usize]));
        let toward = if orthographic { -forward } else { eye - a };
        (b - a).cross(c - a).dot(toward) > 0.0
    };
    // What a ray to a point sees, the point's own faces given.
    let look = |p: Vec3, own: &[u32]| -> Seen {
        if !own.is_empty() && !own.iter().any(|&t| facing(t)) {
            return Seen::Hidden;
        }
        let Some((ray, distance)) = ray_to(camera, aspect, p) else { return Seen::Hidden };
        match pick(&ray) {
            None => Seen::Seen,
            Some((hit_node, primitive, t_hit)) => {
                if t_hit >= distance * (1.0 - 2e-4) - 1e-6 || (hit_node == Some(node) && own.contains(&primitive)) {
                    Seen::Seen
                } else if hit_node == Some(node) {
                    Seen::Hidden
                } else {
                    Seen::Behind
                }
            }
        }
    };
    let screen = |p: Vec3| camera.project(p, aspect).map(|f| Vec2 { x: f.x * rect.width(), y: f.y * rect.height() });
    let mut pieces: Vec<usize> = lines
        .iter()
        .map(|&[a, b]| match (at(a).and_then(screen), at(b).and_then(screen)) {
            (Some(p), Some(q)) => (((q - p).length() / PIECE).ceil() as usize).clamp(1, MOST_PIECES),
            _ => 1,
        })
        .collect();
    if pieces.iter().sum::<usize>() > RAY_BUDGET {
        pieces.iter_mut().for_each(|n| *n = 1);
    }
    for (&[a, b], &n) in lines.iter().zip(&pieces) {
        let (Some(pa), Some(pb)) = (at(a), at(b)) else { continue };
        // The line's own faces: those with both its ends.
        let (ends_a, ends_b) = (around(a), around(b));
        let own: Vec<u32> = ends_a.iter().copied().filter(|t| ends_b.contains(t)).collect();
        let seen = (0..n).map(|k| look(pa + (pb - pa) * ((k as f32 + 0.5) / n as f32), &own)).collect();
        out.lines.insert([a, b], seen);
    }
    for v in points {
        if let Some(p) = at(v) {
            out.points.insert(v, look(p, around(v)));
        }
    }
    out
}

/// Light the hint on the part in the viewport, and the faces picked there
/// at Face level on it, as far as `sight` says the camera sees them.
pub fn draw(ui: &egui::Ui, rect: egui::Rect, camera: &tequila_render_core::camera::Camera, project: &Project, hint: &Hint, sight: &Sight) {
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
    // A line drawn piece by piece, as far as each is seen.
    let line = |[a, b]: [u32; 2], width: f32, colour: egui::Color32| {
        let (Some(pa), Some(pb)) = (at(a), at(b)) else { return };
        let seen = sight.line([a, b]);
        let n = seen.len().max(1);
        for (k, s) in seen.iter().enumerate() {
            let colour = match s {
                Seen::Seen => colour,
                Seen::Behind => colour.gamma_multiply(0.3),
                Seen::Hidden => continue,
            };
            let (p, q) = (pa + (pb - pa) * (k as f32 / n as f32), pa + (pb - pa) * ((k + 1) as f32 / n as f32));
            if let (Some(p), Some(q)) = (screen(p), screen(q)) {
                painter.line_segment([p, q], egui::Stroke::new(width, colour));
            }
        }
    };
    let point = |v: u32| -> Option<(egui::Pos2, f32)> {
        let fade = match sight.points.get(&v).copied().unwrap_or(Seen::Seen) {
            Seen::Seen => 1.0,
            Seen::Behind => 0.3,
            Seen::Hidden => return None,
        };
        at(v).and_then(screen).map(|p| (p, fade))
    };
    // The faces picked at Face level, then what the editor points at.
    let picked: Vec<u32> = project.selection.faces(node).collect();
    if !picked.is_empty() {
        fill(&picked, crate::theme::hue(crate::theme::Hue::Blue).gamma_multiply(0.45), &[]);
    }
    let colour = crate::theme::hue(crate::theme::Hue::Green);
    fill(&hint.triangles, colour.gamma_multiply(0.42), &sight.faces);
    // The seams in orange, as on the layout; the selection over them; what
    // is under the pointer over both.
    let seam = crate::theme::hue(crate::theme::Hue::Orange);
    for &edge in &hint.seams {
        line(edge, 1.5, seam);
    }
    let accent = crate::theme::p().accent;
    for &edge in &hint.selected_edges {
        line(edge, 2.0, accent);
    }
    for &edge in &hint.edges {
        line(edge, 2.5, colour);
    }
    for &v in &hint.selected_vertices {
        if let Some((p, fade)) = point(v) {
            painter.circle_filled(p, 3.5, accent.gamma_multiply(fade));
        }
    }
    let pin = crate::theme::hue(crate::theme::Hue::Violet);
    for &v in &hint.pins {
        if let Some((p, fade)) = point(v) {
            let head = p + egui::vec2(0.0, -11.0);
            painter.line_segment([p, head], egui::Stroke::new(1.5_f32, pin.gamma_multiply(fade)));
            painter.circle_filled(head, 4.0, pin.gamma_multiply(fade));
            painter.circle_stroke(head, 4.0, egui::Stroke::new(1.0_f32, egui::Color32::WHITE.gamma_multiply(fade)));
        }
    }
    for &v in &hint.vertices {
        if let Some((p, fade)) = point(v) {
            painter.circle_filled(p, 4.5, colour.gamma_multiply(fade));
            painter.circle_stroke(p, 4.5, egui::Stroke::new(1.0_f32, egui::Color32::WHITE.gamma_multiply(fade)));
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

    /// A box, its sides facing out, and a sheet in front of the left half
    /// of its front: the project, the box and the sheet.
    fn a_box_behind_a_sheet() -> (Project, NodeId, NodeId) {
        use tequila_scene::command::{reserve_keys, ImportPayload};
        use tequila_scene::{Command, SceneDocument, SceneNode};
        let document = SceneDocument::new();
        let h = 0.5;
        let corners: Vec<Vec3> = (0..8).map(|i| vec3(if i & 1 == 0 { -h } else { h }, if i & 2 == 0 { -h } else { h }, if i & 4 == 0 { -h } else { h })).collect();
        // Each side as two triangles, wound to face out.
        let sides = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let indices: Vec<u32> = sides.iter().flat_map(|[a, b, c, d]| [*a, *b, *c, *a, *c, *d]).collect();
        let cube = tequila_scene::Mesh::new(corners, indices);
        let sheet = tequila_scene::Mesh::new(vec![vec3(-1.0, -1.0, 1.0), vec3(0.0, -1.0, 1.0), vec3(0.0, 1.0, 1.0), vec3(-1.0, 1.0, 1.0)], vec![0, 1, 2, 0, 2, 3]);
        let (meshes, _, ids) = reserve_keys(&document, 2, 0, 2);
        let payload = ImportPayload {
            meshes: vec![(meshes[0], cube), (meshes[1], sheet)],
            textures: Vec::new(),
            materials: Vec::new(),
            nodes: vec![SceneNode::new(ids[0], "Box", NodeKind::Part { mesh: meshes[0] }), SceneNode::new(ids[1], "Sheet", NodeKind::Part { mesh: meshes[1] })],
            roots: ids.clone(),
        };
        let mut project = Project::with_document(document, std::sync::Arc::new(tequila_scene::demo::studio_environment()));
        project.apply(Command::Import { payload: Box::new(payload) }).unwrap();
        (project, ids[0], ids[1])
    }

    /// What a ray meets first among the parts, as the viewport picks.
    fn pick_in(project: &Project, ray: &Ray) -> Option<(Option<NodeId>, u32, f32)> {
        let document = project.document();
        let mut best: Option<(Option<NodeId>, u32, f32)> = None;
        for node in document.tree.iter() {
            let Some(mesh) = node.mesh().and_then(|m| document.geometry.get(m)) else { continue };
            let world = document.tree.world_transform(node.id);
            for t in 0..mesh.triangle_count() {
                let [a, b, c] = mesh.triangle(t).map(|v| world.point(mesh.positions[v as usize]));
                let (e1, e2) = (b - a, c - a);
                let p = ray.direction.cross(e2);
                let det = e1.dot(p);
                if det.abs() < 1e-12 {
                    continue;
                }
                let s = ray.origin - a;
                let u = s.dot(p) / det;
                let q = s.cross(e1);
                let v = ray.direction.dot(q) / det;
                let distance = e2.dot(q) / det;
                if u < 0.0 || v < 0.0 || u + v > 1.0 || distance <= 1e-6 {
                    continue;
                }
                if best.is_none_or(|(_, _, d)| distance < d) {
                    best = Some((Some(node.id), t as u32, distance));
                }
            }
        }
        best
    }

    /// Seen from the front, the box's front edges and corners show; those
    /// behind its own front - its back and the edges running back to it -
    /// do not, though they project inside its outline; and the front edges
    /// behind the sheet show faintly, as seen through it. Seen at a slant
    /// along its top, the edges on its outline still show.
    #[test]
    fn lines_behind_the_part_are_hidden_and_behind_another_faint() {
        let (project, node, _) = a_box_behind_a_sheet();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let corner = |x: f32, y: f32, z: f32| ((x > 0.0) as u32) | (((y > 0.0) as u32) << 1) | (((z > 0.0) as u32) << 2);
        let mut edges = Vec::new();
        for a in 0..8u32 {
            for bit in [1, 2, 4] {
                if a & bit == 0 {
                    edges.push([a, a | bit]);
                }
            }
        }
        let hint = Hint { node: Some(node), seams: edges.clone(), pins: (0..8).collect(), ..Default::default() };
        let camera = tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let sight = sight(&project, &camera, rect, &hint, |ray| pick_in(&project, ray));
        let front = |v: u32| v & 4 != 0;
        let left = |v: u32| v & 1 == 0;
        for [a, b] in &edges {
            let pieces = &sight.lines[&[*a, *b]];
            if front(*a) && front(*b) {
                // The front's left edge is all behind the sheet; the top and
                // bottom run out from behind it at the middle.
                if left(*a) && left(*b) {
                    assert!(pieces.iter().all(|s| *s == Seen::Behind), "{a}-{b}: {pieces:?}");
                } else if !left(*a) && !left(*b) {
                    assert!(pieces.iter().all(|s| *s == Seen::Seen), "{a}-{b}: {pieces:?}");
                } else {
                    assert!(pieces.contains(&Seen::Seen) && pieces.contains(&Seen::Behind), "{a}-{b}: {pieces:?}");
                }
            } else {
                assert!(pieces.iter().all(|s| *s == Seen::Hidden), "behind the box's front, {a}-{b}: {pieces:?}");
            }
        }
        assert_eq!(sight.points[&corner(0.5, 0.5, 0.5)], Seen::Seen);
        assert_eq!(sight.points[&corner(-0.5, 0.5, 0.5)], Seen::Behind);
        assert_eq!(sight.points[&corner(0.5, 0.5, -0.5)], Seen::Hidden);

        // From above and in front, the top and front show, and the top's
        // back edge, on the outline; the bottom's back edge does not.
        let camera = tequila_render_core::camera::Camera::look_at(vec3(0.0, 4.0, 4.0), Vec3::ZERO, Vec3::Y);
        let sight = super::sight(&project, &camera, rect, &hint, |ray| pick_in(&project, ray));
        let top_back = [corner(-0.5, 0.5, -0.5), corner(0.5, 0.5, -0.5)];
        assert!(sight.lines[&top_back].iter().all(|s| *s == Seen::Seen), "{:?}", sight.lines[&top_back]);
        let bottom_back = [corner(-0.5, -0.5, -0.5), corner(0.5, -0.5, -0.5)];
        assert!(sight.lines[&bottom_back].iter().all(|s| *s == Seen::Hidden));
    }
}
