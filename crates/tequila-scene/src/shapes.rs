//! Simple shapes to add to a scene (Scene tab › Add › Shape): a plane, a
//! cube, a cylinder, a sphere and a disc to stand a product on or beside,
//! and a backdrop - a sweep that curves from the floor up into a wall behind
//! the product, as a studio's paper does.
//!
//! Each is a part like any other: its own mesh, with normals and a UV layout
//! (so a texture lies on it as expected), and a material of its own to change.
//! It comes in sized to what is already in the scene and placed beside it, or
//! a backdrop under and behind it, so it is in view and in proportion
//! whatever the product's size; one step to undo, as an import.

use crate::command::{reserve_keys, ImportPayload};
use crate::document::SceneDocument;
use crate::geometry::Mesh;
use crate::material::MaterialDefinition;
use crate::tree::{NodeId, NodeKind, SceneNode};
use tequila_core::math::{vec3, Transform, Vec2, Vec3};

/// The shapes there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Plane,
    Cube,
    Cylinder,
    Sphere,
    Disc,
    /// A floor that curves up into a wall behind the product.
    Backdrop,
}

impl Shape {
    pub const ALL: [Shape; 6] = [Shape::Plane, Shape::Cube, Shape::Cylinder, Shape::Sphere, Shape::Disc, Shape::Backdrop];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Plane => "Plane",
            Shape::Cube => "Cube",
            Shape::Cylinder => "Cylinder",
            Shape::Sphere => "Sphere",
            Shape::Disc => "Disc",
            Shape::Backdrop => "Backdrop",
        }
    }

    /// The shape's mesh, `size` across (a cylinder's and a sphere's
    /// diameter, a cube's side), standing on y = 0 and centred on x and z,
    /// Y up. A backdrop is `size` deep, its wall at the back (−z), three
    /// times as wide, so a three-quarter view still has wall behind the
    /// product to its edges, and seven tenths as tall.
    pub fn mesh(self, size: f32) -> Mesh {
        let s = size.max(1e-4);
        match self {
            Shape::Plane => plane(s),
            Shape::Cube => cube(s),
            Shape::Cylinder => cylinder(s * 0.5, s, 64),
            Shape::Sphere => sphere(s * 0.5, 64, 32),
            Shape::Disc => disc(s * 0.5, 64),
            Shape::Backdrop => sweep(s * 3.0, s, s * 0.7, s * 0.15, 24),
        }
    }
}

/// An Add › Shape: the import that adds `shape` to `document` under
/// `parent` (the root when `None`), with a material of its own, and the new
/// part's id. A shape is half the size of what is in the scene and stands
/// beside it, on the same floor; a backdrop is under and behind it, wide
/// and tall enough to fill the view behind the product. An empty scene gets
/// a 20 cm shape at the origin.
///
/// Each is lifted a thousandth of the scene's size off the floor: the
/// ground is a quad at the floor's height, and a disc, a plane or a
/// backdrop's floor in the same plane fought it for every pixel, in wedges.
pub fn add(document: &SceneDocument, shape: Shape, parent: Option<NodeId>) -> (ImportPayload, NodeId) {
    let bounds = crate::snapshot::visible_bounds(document);
    let empty = bounds.is_empty() || !bounds.diagonal().x.is_finite();
    let (extent, base, centre, back, right) = if empty {
        (0.2, 0.0, Vec3::ZERO, 0.0, 0.0)
    } else {
        let d = bounds.diagonal();
        (d.x.max(d.y).max(d.z).max(1e-3), bounds.min.y, bounds.center(), bounds.min.z, bounds.max.x)
    };
    let lift = extent * 1e-3;
    let (size, place) = match shape {
        _ if empty && shape != Shape::Backdrop => (extent, vec3(0.0, lift, 0.0)),
        Shape::Backdrop => {
            let size = extent * 4.5;
            // The wall a little behind the product, the floor under it and
            // well out in front.
            (size, vec3(centre.x, base + lift, back - extent * 0.4 + size * 0.5))
        }
        _ => {
            let size = extent * 0.5;
            (size, vec3(right + size * 0.75, base + lift, centre.z))
        }
    };
    let (meshes, materials, ids) = reserve_keys(document, 1, 1, 1);
    let mut node = SceneNode::new(ids[0], shape.label(), NodeKind::Part { mesh: meshes[0] });
    node.parent = parent;
    node.transform = Transform::translation(place);
    node.material = Some(materials[0]);
    let payload = ImportPayload {
        meshes: vec![(meshes[0], shape.mesh(size))],
        textures: Vec::new(),
        materials: vec![(materials[0], MaterialDefinition::plastic(format!("{} material", shape.label())))],
        nodes: vec![node],
        roots: vec![ids[0]],
    };
    (payload, ids[0])
}

// ---- The meshes -------------------------------------------------------------

/// Gathers a mesh's vertices and triangles.
#[derive(Default)]
struct Builder {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<Vec2>,
    indices: Vec<u32>,
}

impl Builder {
    fn vertex(&mut self, p: Vec3, n: Vec3, u: f32, v: f32) -> u32 {
        self.positions.push(p);
        self.normals.push(n);
        self.uvs.push(Vec2 { x: u, y: v });
        (self.positions.len() - 1) as u32
    }

    /// A quad of four vertices, counter-clockwise seen from the side they face.
    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.indices.extend_from_slice(&[a, b, c, a, c, d]);
    }

    fn mesh(self) -> Mesh {
        Mesh::new(self.positions, self.indices).with_normals(self.normals).with_uvs(self.uvs)
    }
}

fn plane(s: f32) -> Mesh {
    let mut b = Builder::default();
    let h = s * 0.5;
    let up = Vec3::Y;
    let a = b.vertex(vec3(-h, 0.0, h), up, 0.0, 0.0);
    let c = b.vertex(vec3(h, 0.0, h), up, 1.0, 0.0);
    let d = b.vertex(vec3(h, 0.0, -h), up, 1.0, 1.0);
    let e = b.vertex(vec3(-h, 0.0, -h), up, 0.0, 1.0);
    b.quad(a, c, d, e);
    b.mesh()
}

fn cube(s: f32) -> Mesh {
    let mut b = Builder::default();
    let h = s * 0.5;
    // Each face: its normal and two axes along it, so (u, v) runs across it
    // counter-clockwise seen from outside.
    let faces = [
        (Vec3::X, -Vec3::Z, Vec3::Y),
        (-Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, -Vec3::Z),
        (-Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (-Vec3::Z, -Vec3::X, Vec3::Y),
    ];
    for (n, u, v) in faces {
        let centre = vec3(0.0, h, 0.0) + n * h;
        let corner = |su: f32, sv: f32| centre + u * (su * h) + v * (sv * h);
        let a = b.vertex(corner(-1.0, -1.0), n, 0.0, 0.0);
        let c = b.vertex(corner(1.0, -1.0), n, 1.0, 0.0);
        let d = b.vertex(corner(1.0, 1.0), n, 1.0, 1.0);
        let e = b.vertex(corner(-1.0, 1.0), n, 0.0, 1.0);
        b.quad(a, c, d, e);
    }
    b.mesh()
}

fn cylinder(r: f32, height: f32, segments: usize) -> Mesh {
    let mut b = Builder::default();
    let ring = |i: usize| {
        let t = i as f32 / segments as f32 * std::f32::consts::TAU;
        (t.cos(), t.sin())
    };
    // The side: a seam at angle zero, its two columns of vertices apart so
    // the UV layout runs once round.
    for i in 0..segments {
        let ((c0, s0), (c1, s1)) = (ring(i), ring(i + 1));
        let (n0, n1) = (vec3(c0, 0.0, -s0), vec3(c1, 0.0, -s1));
        let (u0, u1) = (i as f32 / segments as f32, (i + 1) as f32 / segments as f32);
        let a = b.vertex(n0 * r, n0, u0, 0.0);
        let c = b.vertex(n1 * r, n1, u1, 0.0);
        let d = b.vertex(n1 * r + vec3(0.0, height, 0.0), n1, u1, 1.0);
        let e = b.vertex(n0 * r + vec3(0.0, height, 0.0), n0, u0, 1.0);
        b.quad(a, c, d, e);
    }
    cap(&mut b, r, height, Vec3::Y, segments);
    cap(&mut b, r, 0.0, -Vec3::Y, segments);
    b.mesh()
}

/// A flat round cap at height `y`, facing `n` (up or down).
fn cap(b: &mut Builder, r: f32, y: f32, n: Vec3, segments: usize) {
    let centre = b.vertex(vec3(0.0, y, 0.0), n, 0.5, 0.5);
    let first = b.positions.len() as u32;
    for i in 0..=segments {
        let t = i as f32 / segments as f32 * std::f32::consts::TAU;
        let (c, s) = (t.cos(), t.sin());
        b.vertex(vec3(c * r, y, -s * r), n, 0.5 + 0.5 * c, 0.5 + 0.5 * s);
    }
    for i in 0..segments as u32 {
        let (p, q) = (first + i, first + i + 1);
        // Counter-clockwise seen from the side it faces.
        if n.y > 0.0 {
            b.indices.extend_from_slice(&[centre, p, q]);
        } else {
            b.indices.extend_from_slice(&[centre, q, p]);
        }
    }
}

fn disc(r: f32, segments: usize) -> Mesh {
    let mut b = Builder::default();
    cap(&mut b, r, 0.0, Vec3::Y, segments);
    b.mesh()
}

fn sphere(r: f32, segments: usize, rings: usize) -> Mesh {
    let mut b = Builder::default();
    let columns = segments + 1;
    for j in 0..=rings {
        let v = j as f32 / rings as f32;
        let phi = v * std::f32::consts::PI;
        for i in 0..=segments {
            let u = i as f32 / segments as f32;
            let theta = u * std::f32::consts::TAU;
            // From the bottom pole (v = 0) to the top; the poles exactly on
            // the axis, where sin(pi) would leave slivers a ten-millionth wide.
            let n = if j == 0 {
                -Vec3::Y
            } else if j == rings {
                Vec3::Y
            } else {
                vec3(phi.sin() * theta.cos(), -phi.cos(), -phi.sin() * theta.sin())
            };
            b.vertex(n * r + vec3(0.0, r, 0.0), n, u, v);
        }
    }
    for j in 0..rings {
        for i in 0..segments {
            let a = (j * columns + i) as u32;
            let c = a + 1;
            let d = a + columns as u32 + 1;
            let e = a + columns as u32;
            b.quad(a, c, d, e);
        }
    }
    b.mesh()
}

/// A backdrop: a floor `depth` deep coming forward (+z) from the wall's
/// foot, curving up with `radius` into a wall `height` tall, `width` wide,
/// facing the product.
fn sweep(width: f32, depth: f32, height: f32, radius: f32, steps: usize) -> Mesh {
    // The profile in (z, y), from the front of the floor round to the top of
    // the wall, with the way each point faces.
    let r = radius.min(height).min(depth);
    let wall_z = -depth * 0.5;
    let mut profile: Vec<(f32, f32, Vec3)> = vec![(depth * 0.5, 0.0, Vec3::Y)];
    for k in 0..=steps {
        // A quarter turn about (wall_z + r, r), from straight down to
        // straight back.
        let t = k as f32 / steps as f32 * std::f32::consts::FRAC_PI_2;
        let (z, y) = (wall_z + r - r * t.sin(), r - r * t.cos());
        profile.push((z, y, vec3(0.0, t.cos(), t.sin())));
    }
    profile.push((wall_z, height, Vec3::Z));
    let mut along = vec![0.0f32];
    for pair in profile.windows(2) {
        let (dz, dy) = (pair[1].0 - pair[0].0, pair[1].1 - pair[0].1);
        along.push(along.last().unwrap() + (dz * dz + dy * dy).sqrt());
    }
    let total = along.last().copied().unwrap_or(1.0).max(1e-6);
    let mut b = Builder::default();
    let h = width * 0.5;
    for (k, &(z, y, n)) in profile.iter().enumerate() {
        let v = along[k] / total;
        b.vertex(vec3(-h, y, z), n, 0.0, v);
        b.vertex(vec3(h, y, z), n, 1.0, v);
    }
    for k in 0..profile.len() as u32 - 1 {
        let (a, c, d, e) = (2 * k, 2 * k + 1, 2 * k + 3, 2 * k + 2);
        b.quad(a, c, d, e);
    }
    b.mesh()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::CommandBus;

    /// Every shape is a closed-enough, well-formed mesh: its triangles face
    /// the way its normals say (outward, or toward the product for the open
    /// ones), its normals are unit length, it has a UV for every vertex, it
    /// stands on the floor, and it is the size asked for.
    #[test]
    fn every_shape_is_well_formed_and_faces_out() {
        for shape in Shape::ALL {
            let mesh = shape.mesh(2.0);
            assert_eq!(mesh.normals.len(), mesh.positions.len(), "{shape:?}");
            assert_eq!(mesh.uv_sets.first().map(Vec::len), Some(mesh.positions.len()), "{shape:?}");
            assert!(mesh.indices.len() >= 6 && mesh.indices.len() % 3 == 0, "{shape:?}");
            assert!(mesh.normals.iter().all(|n| (n.length() - 1.0).abs() < 1e-3), "{shape:?}: a normal not of unit length");
            let bounds = mesh.bounds();
            assert!(bounds.min.y.abs() < 1e-4, "{shape:?} does not stand on the floor: {:?}", bounds.min.y);
            if shape != Shape::Backdrop {
                let d = bounds.diagonal();
                assert!((d.x - 2.0).abs() < 1e-3, "{shape:?} is {d:?} across, not 2");
            }
            // Each triangle's winding agrees with its vertices' normals.
            for t in mesh.indices.chunks_exact(3) {
                let (a, b, c) = (mesh.positions[t[0] as usize], mesh.positions[t[1] as usize], mesh.positions[t[2] as usize]);
                let face = (b - a).cross(c - a);
                if face.length() < 1e-9 {
                    continue;
                }
                let n = mesh.normals[t[0] as usize] + mesh.normals[t[1] as usize] + mesh.normals[t[2] as usize];
                assert!(face.dot(n) > 0.0, "{shape:?}: a triangle faces against its normals");
            }
        }
    }

    /// Added to a scene, a shape is a part with a mesh and a material of its
    /// own, beside what is there at half its size, on the same floor; a
    /// backdrop is under and behind it. One undo takes it away.
    #[test]
    fn a_shape_comes_in_beside_the_product_in_proportion() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let (product, _) = add(&document, Shape::Cube, None);
        bus.apply(&mut document, Command::Import { payload: Box::new(product) }).unwrap();
        let before = crate::snapshot::visible_bounds(&document);
        assert!((before.diagonal().x - 0.2).abs() < 1e-3, "an empty scene's shape is 20 cm: {:?}", before.diagonal());

        let (payload, sphere) = add(&document, Shape::Sphere, None);
        bus.apply(&mut document, Command::Import { payload: Box::new(payload) }).unwrap();
        let node = document.tree.get(sphere).unwrap();
        assert_eq!(node.name, "Sphere");
        assert!(node.material.is_some() && document.geometry.len() == 2 && document.materials.len() == 2, "a mesh and a material of its own");
        let placed = crate::snapshot::visible_bounds_under(&document, &[sphere]);
        assert!(placed.min.x >= before.max.x, "the sphere overlaps the product: {placed:?} beside {before:?}");
        assert!((placed.diagonal().x - 0.1).abs() < 1e-3, "half the product's size: {:?}", placed.diagonal());
        assert!((placed.min.y - before.min.y).abs() < 1e-3, "on the same floor");
        assert!(placed.min.y > 0.0, "a hair above the ground, not in its plane");

        let (payload, backdrop) = add(&document, Shape::Backdrop, None);
        bus.apply(&mut document, Command::Import { payload: Box::new(payload) }).unwrap();
        let sweep = crate::snapshot::visible_bounds_under(&document, &[backdrop]);
        assert!(sweep.min.z < before.min.z && sweep.max.z > before.max.z, "the floor runs under the product and the wall behind it: {sweep:?}");
        assert!(sweep.max.y > before.max.y && (sweep.min.y - before.min.y).abs() < 1e-3, "taller than the product, from its floor");
        assert!(sweep.min.x < before.min.x - 4.0 * 0.2 && sweep.max.x > before.max.x + 4.0 * 0.2, "wide enough for a three-quarter view: {sweep:?}");

        bus.undo(&mut document).unwrap();
        assert!(!document.tree.contains(backdrop));
    }
}
