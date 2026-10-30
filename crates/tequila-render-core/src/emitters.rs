//! Glowing surfaces as lights.
//!
//! An analytic light is found by next-event estimation. A glowing part, an
//! LED window or a light guide, was found only by a bounce that happened to
//! hit it, and a small bright one was then a rare, heavy hit: noisy light on
//! the parts round it, and some of that light clipped by firefly rejection,
//! which takes such a hit for a firefly.
//!
//! Here every glowing triangle is also a light. The two ways of finding it,
//! a light sample and a bounce that hits it, are weighed against each other
//! by MIS. A light sample chooses a triangle by walking down a tree of them
//! from the point being lit, weighing each branch by its power over its
//! distance, cut down by how it faces the point (§5.3's hierarchy, after
//! Conty Estevez & Kulla 2018, as the analytic lights' is). Then it picks a
//! point on the triangle uniformly.
//!
//! Choosing by power alone is not enough: two parts a few millimetres apart
//! see the near side of each other a hundred times brighter than the far
//! side, and a triangle on the near side chosen no more often than one on the
//! far side is the same rare, heavy sample as before.
//!
//! A part is one of these lights only when its glow is the same all over it
//! and every ray but the camera's sees it. A glow a graph drives, a mixed
//! surface, or a part some kind of ray passes through is still found by the
//! bounces alone, as before, weighed as the only way of finding it. So is
//! everything in a scene with a cutaway, whose caps glow where no triangle
//! is.

use crate::scene::SceneSnapshot;
use tequila_core::ids::InstanceId;
use tequila_core::math::{Aabb, Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::Param;

/// Where a light sample is taken from, which the choice of triangle depends
/// on: the point, its geometric normal turned to face the path, and whether
/// the surface lets light through, so that a glow behind it counts too.
#[derive(Clone, Copy, Debug)]
pub struct Receiver {
    pub position: Vec3,
    pub normal: Vec3,
    pub two_sided: bool,
}

/// The scene's glowing triangles, ready to be chosen.
#[derive(Clone, Debug, Default)]
pub struct Emitters {
    /// The tree; the root is first.
    nodes: Vec<Node>,
    /// Each glowing triangle, as its instance, its index in its mesh and its
    /// area, in the order the tree's leaves name them.
    triangles: Vec<(u32, u32, f32)>,
    /// Per instance, where its triangles start in `leaves`, and `u32::MAX`
    /// for a part that is not one of these lights.
    first: Vec<u32>,
    /// Per triangle of each glowing part, in its mesh's order: the leaf that
    /// holds it, and `u32::MAX` for one too small to have any area.
    leaves: Vec<u32>,
    /// Per instance, what its surface gives off, in nits.
    radiance: Vec<Spectrum>,
}

/// A point chosen on a glowing surface.
#[derive(Clone, Copy, Debug)]
pub struct EmitterSample {
    pub position: Vec3,
    /// The triangle's geometric normal as its winding gives it, the side it
    /// glows from.
    pub normal: Vec3,
    pub instance: InstanceId,
    /// The triangle, by its index in the part's mesh.
    pub primitive: u32,
    /// What it gives off, in nits.
    pub radiance: Spectrum,
    /// The density of choosing this point, per unit area.
    pub density: f32,
}

/// One node of the tree: what the importance needs to know about the
/// triangles beneath it.
#[derive(Clone, Copy, Debug)]
struct Node {
    bounds: Aabb,
    /// The power-weighted mean of the triangles' normals.
    axis: Vec3,
    /// The half-angle about `axis` that their normals spread over.
    theta_o: f32,
    power: f32,
    parent: u32,
    /// The two children, or for a leaf `None` and its triangle in `first`.
    children: Option<(u32, u32)>,
    first: u32,
}

/// A triangle while the tree is built.
struct Glowing {
    instance: u32,
    primitive: u32,
    area: f32,
    power: f32,
    bounds: Aabb,
    centroid: Vec3,
    normal: Vec3,
}

impl Emitters {
    /// The glowing triangles of `scene`'s parts.
    pub fn build(scene: &SceneSnapshot) -> Emitters {
        let count = scene.instances.len();
        let mut out = Emitters {
            first: vec![u32::MAX; count],
            radiance: vec![Spectrum::ZERO; count],
            ..Emitters::default()
        };
        if !scene.cutters().is_empty() {
            return out;
        }
        let mut glowing = Vec::new();
        for (index, instance) in scene.instances.iter().enumerate() {
            let Some(radiance) = steady_glow(scene, index) else { continue };
            let luminance = radiance.luminance();
            if !(luminance > 0.0 && luminance.is_finite()) {
                continue;
            }
            let Some(mesh) = scene.meshes.get(instance.mesh.index()) else { continue };
            let before = glowing.len();
            for primitive in 0..mesh.indices.len() / 3 {
                let [a, b, c] = mesh.triangle(primitive).map(|v| instance.transform.point(mesh.positions[v as usize]));
                let area = 0.5 * (b - a).cross(c - a).length();
                // As the intersector has it, so the side that glows to a
                // light sample is the side that glows to a bounce.
                let normal = instance.transform.normal(mesh.geometric_normal_scaled(primitive)).try_normalize();
                let (true, Some(normal)) = (area > 0.0 && area.is_finite(), normal) else { continue };
                let mut bounds = Aabb::EMPTY;
                for p in [a, b, c] {
                    bounds.extend_point(p);
                }
                glowing.push(Glowing {
                    instance: index as u32,
                    primitive: primitive as u32,
                    area,
                    power: luminance * area,
                    bounds,
                    centroid: (a + b + c) / 3.0,
                    normal,
                });
            }
            if glowing.len() > before {
                out.first[index] = out.leaves.len() as u32;
                out.leaves.extend(std::iter::repeat_n(u32::MAX, mesh.indices.len() / 3));
                out.radiance[index] = radiance;
            }
        }
        if glowing.is_empty() {
            return out;
        }
        out.nodes.reserve(glowing.len() * 2);
        let total = glowing.len();
        out.split(&mut glowing, 0, total, u32::MAX);
        out
    }

    /// The node over `glowing[first..first + count]`, and its subtree.
    fn split(&mut self, glowing: &mut [Glowing], first: usize, count: usize, parent: u32) -> u32 {
        let index = self.nodes.len() as u32;
        let members = &glowing[first..first + count];
        let mut bounds = Aabb::EMPTY;
        let mut axis = Vec3::ZERO;
        let mut power = 0.0;
        for t in members {
            bounds.extend(&t.bounds);
            axis = axis + t.normal * t.power;
            power += t.power;
        }
        let axis = axis.try_normalize().unwrap_or(members[0].normal);
        // A triangle glows from one side, so its own spread is nothing; a
        // cluster's is the widest any member's normal is from the axis.
        let theta_o = members
            .iter()
            .map(|t| t.normal.dot(axis).clamp(-1.0, 1.0).acos())
            .fold(0.0f32, f32::max)
            .min(std::f32::consts::PI);
        self.nodes.push(Node { bounds, axis, theta_o, power, parent, children: None, first: 0 });

        if count == 1 {
            let t = &glowing[first];
            let leaf = self.triangles.len() as u32;
            self.triangles.push((t.instance, t.primitive, t.area));
            self.nodes[index as usize].first = leaf;
            let at = self.first[t.instance as usize] + t.primitive;
            self.leaves[at as usize] = index;
            return index;
        }

        // At the median along the axis the triangles' centres spread widest
        // on. The cluster's own bounds can tie where the centres do not: a
        // panel above another of the same size spans as much across as up,
        // and split across, each side holds some of both and cannot tell a
        // point between them which one it faces.
        let mut centres = Aabb::EMPTY;
        for t in &glowing[first..first + count] {
            centres.extend_point(t.centroid);
        }
        let diagonal = centres.diagonal();
        let along = |p: Vec3| {
            if diagonal.x >= diagonal.y && diagonal.x >= diagonal.z {
                p.x
            } else if diagonal.y >= diagonal.z {
                p.y
            } else {
                p.z
            }
        };
        let middle = count / 2;
        glowing[first..first + count]
            .select_nth_unstable_by(middle, |a, b| along(a.centroid).total_cmp(&along(b.centroid)));
        let left = self.split(glowing, first, middle, index);
        let right = self.split(glowing, first + middle, count - middle, index);
        self.nodes[index as usize].children = Some((left, right));
        index
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The share of a light sample from `at` that the left child of a node
    /// gets, between its two children.
    fn left_share(&self, (left, right): (u32, u32), at: &Receiver) -> f32 {
        let l = importance(&self.nodes[left as usize], at);
        let r = importance(&self.nodes[right as usize], at);
        let total = l + r;
        // Neither child can light this point by the heuristic's reckoning: a
        // coin flip, as the analytic lights' hierarchy falls back to, rather
        // than giving up on light that may still arrive.
        if total > 0.0 && total.is_finite() {
            l / total
        } else {
            0.5
        }
    }

    /// The density per unit area of a light sample from `at` choosing this
    /// point on triangle `primitive` of `instance`: 0 for a part that is not
    /// one of these lights.
    pub fn density(&self, at: &Receiver, instance: InstanceId, primitive: u32) -> f32 {
        let Some(&first) = self.first.get(instance.index()) else { return 0.0 };
        if first == u32::MAX {
            return 0.0;
        }
        let Some(&leaf) = self.leaves.get((first + primitive) as usize) else { return 0.0 };
        if leaf == u32::MAX {
            return 0.0;
        }
        // The probability of the walk down to this leaf, step by step from
        // the leaf up: each step the share its side of the parent got.
        let mut probability = 1.0f32;
        let mut node = leaf;
        while let Some(parent) = self.nodes.get(self.nodes[node as usize].parent as usize) {
            let Some(children) = parent.children else { break };
            let left = self.left_share(children, at);
            probability *= if children.0 == node { left } else { 1.0 - left };
            node = self.nodes[node as usize].parent;
        }
        let area = self.triangles[self.nodes[leaf as usize].first as usize].2;
        probability / area
    }

    /// A point on a glowing triangle, for a light sample from `at`: `u`
    /// chooses the triangle, `uv` the point on it.
    pub fn sample(&self, scene: &SceneSnapshot, at: &Receiver, u: f32, uv: Vec2) -> Option<EmitterSample> {
        let mut node = 0usize;
        let mut probability = 1.0f32;
        let mut u = u.clamp(0.0, 1.0);
        while let Some(children) = self.nodes.get(node)?.children {
            let left = self.left_share(children, at);
            // The same variate rescaled to choose all the way down, as the
            // analytic lights' hierarchy does.
            if u < left {
                u = if left > 0.0 { u / left } else { 0.0 };
                probability *= left;
                node = children.0 as usize;
            } else {
                u = if left < 1.0 { (u - left) / (1.0 - left) } else { 0.0 };
                probability *= 1.0 - left;
                node = children.1 as usize;
            }
            u = u.clamp(0.0, 1.0);
        }
        if !(probability > 0.0) {
            return None;
        }
        let (index, primitive, area) = self.triangles[self.nodes[node].first as usize];
        let instance = scene.instances.get(index as usize)?;
        let mesh = scene.meshes.get(instance.mesh.index())?;
        let [a, b, c] = mesh
            .triangle(primitive as usize)
            .map(|v| instance.transform.point(mesh.positions[v as usize]));
        let bary = tequila_core::sampling::sample_uniform_triangle(uv);
        let position = a * bary.x + b * bary.y + c * (1.0 - bary.x - bary.y);
        let normal = instance.transform.normal(mesh.geometric_normal_scaled(primitive as usize)).try_normalize()?;
        Some(EmitterSample {
            position,
            normal,
            instance: InstanceId(index),
            primitive,
            radiance: self.radiance[index as usize],
            density: probability / area,
        })
    }
}

/// How much the triangles under `node` are worth sampling from `at`: their
/// power over the distance squared, cut down by how far `at` sits outside the
/// cone they glow into and by how obliquely it sees them. The analytic
/// lights' heuristic (`LightNode::importance`), with a triangle's falloff
/// past its cone, a quarter turn, and a surface that lets light through
/// seeing both ways.
fn importance(node: &Node, at: &Receiver) -> f32 {
    if node.power <= 0.0 {
        return 0.0;
    }
    let centre = node.bounds.center();
    let radius = (node.bounds.diagonal().length() * 0.5).max(1e-6);
    let offset = centre - at.position;
    let distance = offset.length();
    let direction = if distance > 1e-9 { offset / distance } else { at.normal };
    // Clamped to the node's own radius, or a point inside a cluster divides
    // by nearly nothing and commits to whichever child it stands in.
    let distance_squared = (distance * distance).max(radius * radius);
    let theta_u = if distance > radius {
        (radius / distance).clamp(-1.0, 1.0).asin()
    } else {
        std::f32::consts::FRAC_PI_2
    };

    let cos_theta = (-direction).dot(node.axis).clamp(-1.0, 1.0);
    let theta_prime = (cos_theta.acos() - node.theta_o - theta_u).max(0.0);
    if theta_prime >= std::f32::consts::FRAC_PI_2 {
        return 0.0;
    }

    let mut cos_incident = direction.dot(at.normal).clamp(-1.0, 1.0);
    if at.two_sided {
        cos_incident = cos_incident.abs();
    }
    let theta_incident = (cos_incident.acos() - theta_u).max(0.0);
    if theta_incident >= std::f32::consts::FRAC_PI_2 {
        return 0.0;
    }
    node.power * theta_prime.cos() * theta_incident.cos() / distance_squared
}

/// What instance `index` gives off, the same all over it, when it is a part
/// that can be one of these lights.
fn steady_glow(scene: &SceneSnapshot, index: usize) -> Option<Spectrum> {
    let instance = &scene.instances[index];
    // A light's stand-in is that light, sampled as one already.
    if instance.light_proxy || !(instance.seen_by_diffuse && instance.seen_by_glossy && instance.seen_by_transmission) {
        return None;
    }
    let material = match scene.program(instance.material) {
        Some(program) => {
            let driven = program
                .driven
                .iter()
                .any(|(param, _)| matches!(param, Param::EmissionLuminance | Param::EmissionColor | Param::Opacity));
            if driven || program.layer.is_some() {
                return None;
            }
            &program.constants
        }
        None => scene.materials.get(instance.material.index())?,
    };
    if !material.is_emissive() {
        return None;
    }
    // A bounce passes through a cut-out surface as often as it is open, so
    // the light it finds is the glow that much of the time.
    Some(material.emission() * material.geometry_opacity.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::Environment;
    use crate::scene::{Instance, Mesh};
    use std::sync::Arc;
    use tequila_core::ids::{MaterialId, MeshId};
    use tequila_core::math::{vec2, vec3, Mat4, Transform};
    use tequila_materials::material_type::MaterialType;
    use tequila_materials::openpbr::Material;

    /// A square `2 * half` across in the plane `y = 0`, in eight triangles,
    /// facing down.
    fn square(half: f32) -> Mesh {
        let mut positions = Vec::new();
        for z in 0..3 {
            for x in 0..3 {
                positions.push(vec3(-half + half * x as f32, 0.0, -half + half * z as f32));
            }
        }
        let mut indices = Vec::new();
        for z in 0..2u32 {
            for x in 0..2u32 {
                let a = z * 3 + x;
                indices.extend_from_slice(&[a, a + 1, a + 4, a, a + 4, a + 3]);
            }
        }
        Mesh::new(positions, indices)
    }

    fn glow(luminance: f32) -> Material {
        Material {
            emission_color: Spectrum::ONE,
            emission_luminance: luminance,
            ..MaterialType::Emissive.defaults()
        }
    }

    /// A square at 2 nits above the origin, a bigger one at 1 nit off to
    /// the side, and three parts that are not lights: a matte one, a light's
    /// stand-in, and a glow that diffuse bounces pass through.
    fn scene() -> SceneSnapshot {
        let up = |y: f32| Transform::translation(vec3(0.0, y, 0.0));
        let mut unseen = Instance::new(InstanceId(4), MeshId(1), MaterialId(1)).with_transform(up(1.0));
        unseen.seen_by_diffuse = false;
        SceneSnapshot::new(
            vec![Arc::new(square(0.5)), Arc::new(square(1.0))],
            vec![
                Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(up(1.0)),
                Instance::new(InstanceId(1), MeshId(1), MaterialId(1)).with_transform(Transform::translation(vec3(4.0, 2.0, 0.0))),
                Instance::new(InstanceId(2), MeshId(1), MaterialId(2)).with_transform(up(1.0)),
                Instance::new(InstanceId(3), MeshId(1), MaterialId(1)).with_transform(up(1.0)).as_light_proxy(),
                unseen,
            ],
            vec![glow(2.0), glow(1.0), MaterialType::Diffuse.defaults()],
            Arc::new(Environment::uniform(Spectrum::ZERO)),
        )
    }

    fn below() -> Receiver {
        Receiver { position: Vec3::ZERO, normal: Vec3::Y, two_sided: false }
    }

    /// The probability a light sample gives a triangle is the probability
    /// it is chosen with, and over every glowing triangle they add up to one
    /// - which is what lets a bounce that hits a glow weigh itself against
    /// the light sample that could have found the same point.
    #[test]
    fn a_triangle_is_chosen_as_often_as_its_density_says() {
        let scene = scene();
        let emitters = scene.emitters();
        let at = below();
        let mut chosen = std::collections::HashMap::<(u32, u32), u32>::new();
        let n = 40_000;
        for k in 0..n {
            let s = emitters.sample(&scene, &at, (k as f32 + 0.5) / n as f32, vec2(0.3, 0.6)).unwrap();
            assert!((s.density - emitters.density(&at, s.instance, s.primitive)).abs() <= 1e-5 * s.density);
            *chosen.entry((s.instance.0, s.primitive)).or_default() += 1;
        }
        // Each square's eight triangles are each half its area over four.
        let mut sum = 0.0;
        for (instance, half) in [(0u32, 0.5f32), (1, 1.0)] {
            for primitive in 0..8 {
                let probability = emitters.density(&at, InstanceId(instance), primitive) * half * half / 2.0;
                sum += probability;
                let share = chosen.get(&(instance, primitive)).copied().unwrap_or(0) as f32 / n as f32;
                assert!(
                    (share - probability).abs() < 1e-3 + 1e-2 * probability,
                    "triangle {instance}/{primitive}: chosen {share} of the time, its density says {probability}"
                );
            }
        }
        assert!((sum - 1.0).abs() < 1e-3, "the choices add up to {sum}");
        for other in 2..5 {
            assert_eq!(emitters.density(&at, InstanceId(other), 0), 0.0, "part {other} was made a light");
            assert!(!chosen.keys().any(|&(i, _)| i == other), "part {other} was chosen");
        }
    }

    /// A glow is chosen by how much light it gives the point: the small
    /// square straight above, though dimmer in all, far more often than the
    /// big one off to the side, and the side one more often from a point
    /// beneath it.
    #[test]
    fn the_nearer_glow_is_chosen_more_often() {
        let scene = scene();
        let emitters = scene.emitters();
        let share = |at: Receiver| {
            let n = 4000;
            (0..n)
                .filter(|k| emitters.sample(&scene, &at, (*k as f32 + 0.5) / n as f32, vec2(0.5, 0.5)).unwrap().instance == InstanceId(0))
                .count() as f32
                / n as f32
        };
        let above = share(below());
        let aside = share(Receiver { position: vec3(4.0, 0.0, 0.0), ..below() });
        assert!(above > 0.8, "the square overhead was chosen {above} of the time");
        assert!(aside < 0.2, "from beneath the other one, the first was chosen {aside} of the time");
    }

    /// A surface that lets light through counts a glow behind it: with the
    /// big square moved under the point and turned to face up at it, an
    /// opaque surface facing up all but never chooses it, and glass gives it
    /// its share.
    #[test]
    fn glass_counts_a_glow_behind_it() {
        let mut scene = scene();
        scene.instances[1].transform =
            Transform::from_matrix(Mat4::translation(vec3(0.0, -1.0, 0.0)) * Mat4::scale(vec3(1.0, -1.0, 1.0))).unwrap();
        let emitters = scene.emitters();
        let opaque = below();
        let glass = Receiver { two_sided: true, ..opaque };
        // Each of its eight triangles is a square metre over two.
        let underneath = |at: &Receiver| (0..8).map(|p| emitters.density(at, InstanceId(1), p) * 0.5).sum::<f32>();
        assert!(underneath(&opaque) < 0.02, "an opaque surface chose the glow behind it {} of the time", underneath(&opaque));
        assert!(underneath(&glass) > 0.4, "glass chose the glow behind it {} of the time", underneath(&glass));
    }

    /// A copy works its glowing parts out afresh, so a copy whose part has
    /// moved samples the part where it now is.
    #[test]
    fn a_copy_samples_its_parts_where_they_are() {
        let scene = scene();
        assert!(scene.emitters().sample(&scene, &below(), 0.1, vec2(0.5, 0.5)).is_some());
        let mut moved = scene.clone();
        moved.instances[0].transform = Transform::translation(vec3(0.0, 5.0, 0.0));
        let sample = moved.emitters().sample(&moved, &below(), 0.1, vec2(0.5, 0.5)).unwrap();
        assert_eq!(sample.instance, InstanceId(0));
        assert!((sample.position.y - 5.0).abs() < 1e-5, "sampled at {:?}", sample.position);
    }

    /// A mirrored part glows from the side the intersector says it does.
    #[test]
    fn a_mirrored_glow_keeps_its_side() {
        let mut scene = scene();
        scene.instances[0].transform = Transform::from_matrix(Mat4::translation(vec3(0.0, 1.0, 0.0)) * Mat4::scale(vec3(-1.0, 1.0, 1.0))).unwrap();
        let mesh = &scene.meshes[0];
        let intersector_normal = scene.instances[0].transform.normal(mesh.geometric_normal_scaled(0)).normalize();
        let sample = scene.emitters().sample(&scene, &below(), 0.01, vec2(0.5, 0.5)).unwrap();
        assert_eq!(sample.instance, InstanceId(0));
        assert!(sample.normal.dot(intersector_normal) > 0.999);
    }

    /// Light source materials or not, a scene with a cutaway leaves its
    /// glows to the bounces: a cut part's cap glows where no triangle is.
    #[test]
    fn a_cutaway_leaves_glows_to_the_bounces() {
        let cut = scene().with_cutters(vec![crate::cutaway::Cutter::new(crate::cutaway::CutterShape::Plane, Transform::default())]);
        assert!(cut.emitters().is_empty());
        assert_eq!(cut.emitters().density(&below(), InstanceId(0), 0), 0.0);
    }
}
