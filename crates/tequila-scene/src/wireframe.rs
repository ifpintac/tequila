//! The lines a wireframe render draws (`RenderKind::Wireframe`), and the
//! picture they make once drawn.
//!
//! Gathered here, where the meshes and their feature edges are known. Which
//! stretches of them the camera sees is the renderer's to say
//! (`tequila_render_cpu::wireframe`), by asking the scene itself: an edge is
//! hidden where something lies between it and the camera, which a ray finds
//! exactly at any size of image, where a depth buffer would have to guess
//! at the edges it was drawn from.

use crate::measure::FeatureEdges;
use crate::render::{WireEdges, WireframeSettings};
use std::collections::{HashMap, HashSet};
use tequila_core::math::{vec3, Mat4, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_render_core::camera::{Camera, Projection};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};

/// Whether an instance is drawn: what the viewport's Wireframe draws, the
/// parts the camera sees and not the ground under them.
pub fn drawn(instance: &Instance) -> bool {
    instance.visible_to_camera && instance.shadow_catcher.is_none()
}

/// The scene as the wireframe's visibility rays see it: only what is drawn,
/// so the ground does not hide the edges that sit on it.
pub fn visible_scene(scene: &SceneSnapshot) -> SceneSnapshot {
    let mut drawn_only = scene.clone();
    drawn_only.instances.retain(drawn);
    drawn_only
}

/// A mesh's edges once each, with the faces on either side of them.
struct Topology {
    /// Each vertex's position-welded index: a mesh split for its normals has
    /// two vertices at every crease, and one edge there, not two.
    points: Vec<Vec3>,
    /// Every edge, by welded end points.
    edges: Vec<(u32, u32)>,
    /// The edges between exactly two faces, and those faces' normals, which
    /// say from where the edge is part of the outline.
    shared: Vec<(u32, u32, Vec3, Vec3)>,
}

impl Topology {
    fn of(mesh: &Mesh) -> Topology {
        let size = mesh.bounds().diagonal().length().max(1e-12);
        let quantum = size * 1e-6;
        let mut ids: HashMap<(i64, i64, i64), u32> = HashMap::new();
        let mut points = Vec::new();
        let weld: Vec<u32> = mesh
            .positions
            .iter()
            .map(|p| {
                let key = ((p.x / quantum).round() as i64, (p.y / quantum).round() as i64, (p.z / quantum).round() as i64);
                *ids.entry(key).or_insert_with(|| {
                    points.push(*p);
                    (points.len() - 1) as u32
                })
            })
            .collect();
        let mut faces: HashMap<(u32, u32), Vec<Vec3>> = HashMap::new();
        for t in 0..mesh.triangle_count() {
            let [a, b, c] = mesh.triangle(t).map(|v| weld[v as usize]);
            if a == b || b == c || c == a {
                continue;
            }
            let normal = (points[b as usize] - points[a as usize]).cross(points[c as usize] - points[a as usize]);
            for (p, q) in [(a, b), (b, c), (c, a)] {
                faces.entry((p.min(q), p.max(q))).or_default().push(normal);
            }
        }
        let mut edges: Vec<(u32, u32)> = faces.keys().copied().collect();
        // Sorted, so the same scene draws the same lines in the same order.
        edges.sort_unstable();
        let shared = edges
            .iter()
            .filter_map(|&(a, b)| match faces[&(a, b)].as_slice() {
                [n1, n2] => Some((a, b, *n1, *n2)),
                _ => None,
            })
            .collect();
        Topology { points, edges, shared }
    }
}

/// Every line the wireframe draws, in world space, as pairs of points: each
/// drawn part's edges - every triangle's, or its model edges and outline -
/// placed where the part is.
pub fn lines(scene: &SceneSnapshot, camera: &Camera, edges: WireEdges) -> Vec<[Vec3; 2]> {
    let mut topologies: HashMap<usize, Topology> = HashMap::new();
    let mut models: HashMap<usize, Vec<Vec3>> = HashMap::new();
    let mut out = Vec::new();
    let eye = camera.position();
    let forward = camera.forward();
    let orthographic = matches!(camera.projection, Projection::Orthographic { .. });
    for instance in scene.instances.iter().filter(|i| drawn(i)) {
        let mesh = scene.mesh(instance.mesh);
        let key = mesh as *const Mesh as usize;
        let to_world = instance.transform.matrix;
        let topology = topologies.entry(key).or_insert_with(|| Topology::of(mesh));
        let world = |p: Vec3| to_world.transform_point(p);
        match edges {
            WireEdges::Mesh => {
                for &(a, b) in &topology.edges {
                    out.push([world(topology.points[a as usize]), world(topology.points[b as usize])]);
                }
            }
            WireEdges::Model => {
                let model = models.entry(key).or_insert_with(|| {
                    if mesh.edge_lines.len() >= 2 {
                        mesh.edge_lines.clone()
                    } else {
                        FeatureEdges::of(mesh).segments(mesh)
                    }
                });
                for pair in model.chunks_exact(2) {
                    out.push([world(pair[0]), world(pair[1])]);
                }
                out.extend(outline(topology, &instance.transform.inverse, eye, forward, orthographic).map(|[a, b]| [world(a), world(b)]));
            }
        }
    }
    out
}

/// The edges a part's outline runs along from where the camera is: those
/// between a face turned toward it and one turned away. Worked out in the
/// part's own space, where which side of a face a point is on is the same
/// question it is in the world's.
fn outline<'a>(topology: &'a Topology, to_local: &Mat4, eye: Vec3, forward: Vec3, orthographic: bool) -> impl Iterator<Item = [Vec3; 2]> + 'a {
    let eye = to_local.transform_point(eye);
    let toward = -to_local.transform_vector(forward);
    topology.shared.iter().filter_map(move |&(a, b, n1, n2)| {
        let (pa, pb) = (topology.points[a as usize], topology.points[b as usize]);
        let view = if orthographic { toward } else { eye - (pa + pb) * 0.5 };
        let (s1, s2) = (n1.dot(view), n2.dot(view));
        (s1 > 0.0 && s2 <= 0.0 || s1 <= 0.0 && s2 > 0.0).then_some([pa, pb])
    })
}

/// How many distinct edges a set of lines holds, ignoring direction: for the
/// tests, and for saying how much was drawn.
pub fn distinct(lines: &[[Vec3; 2]]) -> usize {
    let key = |p: Vec3| (p.x.to_bits(), p.y.to_bits(), p.z.to_bits());
    let set: HashSet<_> = lines
        .iter()
        .map(|[a, b]| {
            let (ka, kb) = (key(*a), key(*b));
            (ka.min(kb), ka.max(kb))
        })
        .collect();
    set.len()
}

/// The finished picture from the drawn lines' coverage: `front` the edges
/// the camera sees, `behind` the hidden ones, each 0 to 1 per pixel. The
/// colour comes back premultiplied by the alpha, as a render's does; with
/// `transparent` the background is left out and the alpha is the lines'.
///
/// Mixed in display values, as a drawing program mixes them: in linear
/// light a thin dark line on white comes out lighter than it was drawn.
pub fn composite(
    width: usize,
    height: usize,
    front: &[f32],
    behind: &[f32],
    settings: &WireframeSettings,
    transparent: bool,
) -> (Vec<Spectrum>, Vec<f32>) {
    let strength = settings.hidden.strength();
    let line = settings.line_colour;
    let mut colour = Vec::with_capacity(width * height);
    let mut alpha = Vec::with_capacity(width * height);
    for y in 0..height {
        let along = if height > 1 { y as f32 / (height - 1) as f32 } else { 0.0 };
        let ground: [f32; 3] =
            std::array::from_fn(|c| settings.background_top[c] + (settings.background_bottom[c] - settings.background_top[c]) * along);
        for x in 0..width {
            let i = y * width + x;
            let near = front[i].clamp(0.0, 1.0);
            let far = behind[i].clamp(0.0, 1.0) * strength;
            let a = near + far * (1.0 - near);
            if transparent {
                colour.push(linear(line) * a);
                alpha.push(a);
            } else {
                colour.push(linear(std::array::from_fn(|c| ground[c] + (line[c] - ground[c]) * a)));
                alpha.push(1.0);
            }
        }
    }
    (colour, alpha)
}

/// A display sRGB colour as the light it stands for.
fn linear(display: [f32; 3]) -> Spectrum {
    let decode = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    Spectrum::from_linear_srgb(vec3(decode(display[0]), decode(display[1]), decode(display[2])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{HiddenLines, WireframeSettings};
    use tequila_core::ids::{InstanceId, MaterialId, MeshId};
    use tequila_render_core::environment::Environment;
    use tequila_render_core::material::Material;
    use std::sync::Arc;

    /// A unit cube split at every crease, as a CAD mesh is: four vertices a
    /// face, twenty-four in all.
    fn cube() -> Mesh {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        let faces: [[Vec3; 4]; 6] = [
            [vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(0.0, 1.0, 1.0)],
            [vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(1.0, 1.0, 0.0)],
            [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 1.0), vec3(0.0, 1.0, 0.0)],
            [vec3(1.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(1.0, 1.0, 1.0)],
            [vec3(0.0, 1.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)],
            [vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0)],
        ];
        for quad in faces {
            let base = positions.len() as u32;
            positions.extend_from_slice(&quad);
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        Mesh::new(positions, indices)
    }

    fn scene_of(mesh: Mesh) -> SceneSnapshot {
        SceneSnapshot::new(
            vec![Arc::new(mesh)],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
            vec![Material::diffuse(Spectrum::splat(0.5))],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        )
    }

    #[test]
    fn every_triangle_edge_is_drawn_once_across_the_split_creases() {
        let scene = scene_of(cube());
        let camera = Camera::look_at(vec3(3.0, 2.5, 4.0), Vec3::splat(0.5), Vec3::Y);
        let lines = lines(&scene, &camera, WireEdges::Mesh);
        // Twelve edges of the cube and a diagonal across each of six faces:
        // the crease edges are shared by two faces' separate vertices.
        assert_eq!(lines.len(), 18);
        assert_eq!(distinct(&lines), 18);
    }

    #[test]
    fn model_edges_leave_the_diagonals_out() {
        let scene = scene_of(cube());
        let camera = Camera::look_at(vec3(3.0, 2.5, 4.0), Vec3::splat(0.5), Vec3::Y);
        let lines = lines(&scene, &camera, WireEdges::Model);
        // The twelve creases, and the outline from this corner view - six of
        // those same edges - drawn over them; no face diagonal.
        assert_eq!(distinct(&lines), 12);
        for [a, b] in &lines {
            let changed = [a.x != b.x, a.y != b.y, a.z != b.z].iter().filter(|c| **c).count();
            assert_eq!(changed, 1, "a cube's edges run along one axis: {a:?} to {b:?}");
        }
    }

    #[test]
    fn a_smooth_part_is_outlined_where_it_turns_away() {
        // A tessellated cylinder along Y, seen side on from +Z: no creases
        // along its sides, so its outline is what draws them - the two
        // facet edges at its left and right, and only those.
        let segments = 32;
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for i in 0..segments {
            let angle = i as f32 / segments as f32 * std::f32::consts::TAU;
            positions.push(vec3(angle.cos(), 0.0, angle.sin()));
            positions.push(vec3(angle.cos(), 2.0, angle.sin()));
        }
        for i in 0..segments as u32 {
            let (a, b) = (i * 2, ((i + 1) % segments as u32) * 2);
            indices.extend_from_slice(&[a, b + 1, b, a, a + 1, b + 1]);
        }
        let scene = scene_of(Mesh::new(positions, indices));
        let camera = Camera::look_at(vec3(0.0, 1.0, 10.0), vec3(0.0, 1.0, 0.0), Vec3::Y);
        let topology = Topology::of(scene.mesh(MeshId(0)));
        let sides: Vec<_> = outline(&topology, &Mat4::IDENTITY, camera.position(), camera.forward(), false)
            .filter(|[a, b]| (a.y - b.y).abs() > 1.0)
            .collect();
        assert_eq!(sides.len(), 2, "{sides:?}");
        for [a, _] in sides {
            assert!(a.x.abs() > 0.95, "the outline is at the cylinder's sides, not across its front: {a:?}");
        }
    }

    #[test]
    fn the_ground_is_neither_drawn_nor_in_the_way() {
        let mut scene = scene_of(cube());
        let mut ground = Instance::new(InstanceId(1), MeshId(0), MaterialId(0));
        ground.shadow_catcher = Some(Default::default());
        scene.instances.push(ground);
        let mut hidden = Instance::new(InstanceId(2), MeshId(0), MaterialId(0));
        hidden.visible_to_camera = false;
        scene.instances.push(hidden);
        let camera = Camera::look_at(vec3(3.0, 2.5, 4.0), Vec3::splat(0.5), Vec3::Y);
        assert_eq!(lines(&scene, &camera, WireEdges::Mesh).len(), 18, "the cube's alone");
        assert_eq!(visible_scene(&scene).instances.len(), 1);
    }

    #[test]
    fn a_line_comes_out_the_colour_it_was_given() {
        let settings = WireframeSettings { hidden: HiddenLines::Faint, ..WireframeSettings::LIGHT };
        let (colour, alpha) = composite(3, 1, &[1.0, 0.0, 0.0], &[0.0, 1.0, 0.0], &settings, false);
        assert_eq!(alpha, vec![1.0; 3]);
        let display = |s: Spectrum| {
            let v = s.to_linear_srgb();
            let encode = |v: f32| if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
            [encode(v.x), encode(v.y), encode(v.z)]
        };
        for (got, want) in display(colour[0]).iter().zip(settings.line_colour) {
            assert!((got - want).abs() < 1e-3, "{got} against {want}");
        }
        // Hidden, a fifth of the way from white to the line.
        let faint = display(colour[1]);
        assert!((faint[0] - (1.0 + (0.1 - 1.0) * 0.2)).abs() < 1e-3, "{faint:?}");
        assert!(display(colour[2]).iter().all(|v| (v - 1.0).abs() < 1e-3), "the paper where nothing was drawn");

        let (colour, alpha) = composite(2, 1, &[0.5, 0.0], &[0.0, 0.0], &settings, true);
        assert_eq!(alpha, vec![0.5, 0.0]);
        assert_eq!(colour[1], Spectrum::ZERO, "no background in a file with alpha");
    }
}
