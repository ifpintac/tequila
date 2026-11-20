//! The scene's glowing triangles on the card (`docs/07` R3 tier A,
//! "emissive triangles and their tree"): `tequila_render_core::emitters`'s
//! tree exactly as the processor built it, not built again here (a tie its
//! sort breaks one way could break the other), for `emitters.wgsl` to
//! choose a triangle from a point, to pick a point on it, and to say how
//! likely a light sample was to choose a point a bounce hit.
//!
//! Whatever the processor works out each time from a node or a triangle
//! alone comes worked out, by the processor's own arithmetic, so it is the
//! processor's number: a node's centre and radius (`importance`), a leaf's
//! world corners and normal (`Emitters::sample`).

use bytemuck::{Pod, Zeroable};
use tequila_core::ids::InstanceId;
use tequila_render_core::scene::SceneSnapshot;

/// `emitters.wgsl`'s `GpuEmitterNode`: a node of the tree, and for a leaf
/// its triangle.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuEmitterNode {
    /// The bounds' centre, and in `[3]` the radius `importance` takes:
    /// half the diagonal, at least 1e-6.
    pub centre: [f32; 4],
    /// The power-weighted mean of the triangles' normals, and in `[3]` the
    /// half-angle they spread over (`theta_o`).
    pub axis: [f32; 4],
    /// A leaf's triangle's corners in world space, as `Emitters::sample`
    /// places them; in `a[3]` its area.
    pub a: [f32; 4],
    pub b: [f32; 4],
    pub c: [f32; 4],
    /// A leaf's triangle's normal, the side it glows from.
    pub normal: [f32; 4],
    /// What a leaf's part gives off, in nits, opacity included.
    pub radiance: [f32; 4],
    pub power: f32,
    /// The node above, [`EMITTER_NONE`] at the root.
    pub parent: u32,
    /// The children; [`EMITTER_NONE`] for a leaf.
    pub left: u32,
    pub right: u32,
    /// A leaf's part, by its place among the card's instances, and the
    /// triangle's index in its mesh.
    pub instance: u32,
    pub primitive: u32,
    pub _pad: [u32; 2],
}

/// No node, no leaf, no part.
pub const EMITTER_NONE: u32 = u32::MAX;

/// The tree as the card reads it: the nodes (binding 18), the leaf of each
/// glowing part's triangles (binding 19), and per instance of the snapshot
/// where its triangles start among those, [`EMITTER_NONE`] for a part that
/// is not one of these lights (`GpuInstance::emitter`).
#[derive(Clone, Debug, Default)]
pub struct EmitterRecords {
    pub nodes: Vec<GpuEmitterNode>,
    pub leaves: Vec<u32>,
    pub first: Vec<u32>,
}

impl EmitterRecords {
    /// `scene`'s glowing triangles, its instances numbered among the card's
    /// by `slot` (an instance's place in the snapshot to its place among
    /// the instances the card traces; every glowing part has triangles, so
    /// is traced).
    pub fn of(scene: &SceneSnapshot, slot: &dyn Fn(u32) -> u32) -> EmitterRecords {
        let emitters = scene.emitters();
        let mut nodes = Vec::with_capacity(emitters.nodes().len());
        for node in emitters.nodes() {
            let centre = node.bounds.center();
            let radius = (node.bounds.diagonal().length() * 0.5).max(1e-6);
            let mut record = GpuEmitterNode {
                centre: [centre.x, centre.y, centre.z, radius],
                axis: [node.axis.x, node.axis.y, node.axis.z, node.theta_o],
                power: node.power,
                parent: node.parent,
                left: node.children.map_or(EMITTER_NONE, |c| c.0),
                right: node.children.map_or(EMITTER_NONE, |c| c.1),
                instance: EMITTER_NONE,
                primitive: 0,
                ..GpuEmitterNode::default()
            };
            if let Some((index, primitive, area)) = node.triangle {
                let instance = &scene.instances[index as usize];
                let mesh = &scene.meshes[instance.mesh.index()];
                // As Emitters::sample has them.
                let [a, b, c] = mesh.triangle(primitive as usize).map(|v| instance.transform.point(mesh.positions[v as usize]));
                let normal = instance
                    .transform
                    .normal(mesh.geometric_normal_scaled(primitive as usize))
                    .try_normalize()
                    .unwrap_or(tequila_core::math::Vec3::ZERO);
                let glow = emitters.radiance(InstanceId(index));
                record.a = [a.x, a.y, a.z, area];
                record.b = [b.x, b.y, b.z, 0.0];
                record.c = [c.x, c.y, c.z, 0.0];
                record.normal = [normal.x, normal.y, normal.z, 0.0];
                record.radiance = [glow.r, glow.g, glow.b, 0.0];
                record.instance = slot(index);
                record.primitive = primitive;
            }
            nodes.push(record);
        }
        EmitterRecords { nodes, leaves: emitters.leaves().to_vec(), first: emitters.first_leaves().to_vec() }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
