//! Bump and normal mapping at a hit (`docs/02-MATERIAL-SYSTEM.md` §5.4).
//!
//! The material graph's Bump socket says how the surface is textured; this is
//! where that becomes a shading normal. Two readings of the one socket:
//!
//! - **A normal map** — a texture tagged [`MapRole::Normal`] wired straight
//!   in. Its texels are tangent-space normals, decoded against the frame the
//!   mesh's UVs run in. This is what every glTF, and most other exchange
//!   formats, carry for surface detail.
//! - **A height**, in scene units — anything else. Typically a pattern or a
//!   height map through a Bump node, which gives the 0-to-1 value its depth.
//!   The slope is measured by evaluating just the bump's part of the graph a
//!   little either side of the point, along the surface, in world space.
//!
//! Measuring in world space is what makes a bump mean the same thing on every
//! part: half a millimetre deep is half a millimetre on a watch and on a car,
//! however each was scaled on import and however its UVs were laid out.
//!
//! The step is a tenth of the ray's footprint, [`BUMP_FILTER_WIDTH`]: what
//! Cycles' bump node takes by default, and so what Tequila's Cycles
//! translation renders. Detail near a pixel's size is measured where each
//! sample lands, and the pixel's samples average it, in both engines alike.
//! A whole footprint either side, as it was, smoothed the slope over two
//! pixels: a 3D print's 0.2 mm layer lines, four pixels apart in a close-up,
//! shaded a third shallower here than in Cycles and in Blender.

use crate::intersect::Hit;
use crate::material::Material;
use crate::scene::SceneSnapshot;
use crate::shade::{shade, Registers, TextureTable};
use tequila_core::math::{vec2, vec3, Vec3};
use tequila_materials::graph::{CompiledGraph, Instruction, Register};
use tequila_materials::material_type::Param;
use tequila_texture::projection::SurfacePoint;
use tequila_texture::texture::MapRole;
use tequila_texture::TangentFrame;
use std::sync::Arc;

/// The share of the ray's footprint a bump's slope is measured across.
pub const BUMP_FILTER_WIDTH: f32 = 0.1;

/// What drives a material's Bump socket.
#[derive(Debug)]
pub enum BumpSource {
    /// A tangent-space normal map, sampled by the material's own program into
    /// `register`, from `texture` through `mapping`.
    NormalMap {
        register: Register,
        texture: tequila_materials::graph::TextureId,
        mapping: Box<tequila_texture::projection::Mapping>,
    },
    /// A height in scene units, which `program` — the material's program cut
    /// down to what the bump needs — computes into `register`.
    Height {
        program: Arc<CompiledGraph>,
        register: Register,
    },
}

impl BumpSource {
    /// How a compiled material bumps its surface, or `None` when it does not.
    pub fn of(program: &CompiledGraph, textures: &TextureTable) -> Option<BumpSource> {
        let driven = |wanted: Param| program.driven.iter().find(|(param, _)| *param == wanted).map(|(_, r)| *r);
        let Some(register) = driven(Param::Bump) else {
            // §4.4's bump-only displacement: with no detail to subdivide to,
            // a Displacement height shades as the bump it would have cast.
            let register = driven(Param::Displacement)?;
            if program.constants.displacement_detail > 0.0 {
                return None;
            }
            return Some(BumpSource::Height {
                program: Arc::new(program.restricted_to(&[Param::Displacement])),
                register,
            });
        };
        // The last instruction to write the register is the value that
        // reaches the socket.
        let writer = program
            .instructions
            .iter()
            .rev()
            .find(|instruction| instruction.destination() == register)?;
        if let Instruction::Sample { texture, mapping, .. } = writer {
            match textures.get(*texture) {
                Some(loaded) if loaded.role == MapRole::Normal => {
                    return Some(BumpSource::NormalMap { register, texture: *texture, mapping: mapping.clone() });
                }
                Some(_) => {}
                // Not loaded: its fallback is a number, not a normal, and
                // decoding one would tilt the whole surface. Flat until the
                // file arrives.
                None => return None,
            }
        }
        Some(BumpSource::Height {
            program: Arc::new(program.restricted_to(&[Param::Bump])),
            register,
        })
    }
}

/// The shading normal with the material's bump applied, in world space.
///
/// `main` holds the registers the material's own program just filled at
/// `point`; `registers` and `scratch` are for evaluating the height either
/// side of it. Falls back to the unbumped normal wherever the answer would
/// face away from the geometry — a bump that turns a surface inside out is
/// the shading-normal leak §3.1 guards against, made worse.
#[allow(clippy::too_many_arguments)]
pub fn bumped_normal(
    source: &BumpSource,
    scene: &SceneSnapshot,
    hit: &Hit,
    point: &SurfacePoint,
    shading_normal: Vec3,
    geometric_normal: Vec3,
    main: &Registers,
    registers: &mut Registers,
    scratch: &mut Material,
) -> Vec3 {
    let n = shading_normal;
    let derivatives = scene.uv_derivatives(hit);
    let bumped = match source {
        // Laid down by a projection of its own (a texture pack's tri-planar,
        // a planar decal): decoded in that projection's frames, not the
        // mesh's UVs, which it does not follow.
        BumpSource::NormalMap { texture, mapping, .. }
            if mapping.projection != tequila_texture::projection::Projection::Uv =>
        {
            let Some(loaded) = scene.textures.get(*texture) else {
                return n;
            };
            let Some(local) = loaded.sample_normal_with(mapping, point) else {
                return n;
            };
            scene.instance(hit.instance).transform.normal(local).try_normalize().unwrap_or(n)
        }
        BumpSource::NormalMap { register, .. } => {
            // A normal map is in the frame the UVs run in; without UVs it has
            // nothing to be in.
            let Some((dpdu, dpdv)) = derivatives else {
                return n;
            };
            let (Some(tangent), Some(down)) = (
                (dpdu - n * n.dot(dpdu)).try_normalize(),
                (dpdv - n * n.dot(dpdv)).try_normalize(),
            ) else {
                return n;
            };
            // Green is "up" in the image (glTF's and OpenGL's convention), and
            // v runs *down* the image here — texture row 0 is v = 0, as glTF
            // stores it — so up is towards decreasing v. Taken from dpdv
            // itself rather than a cross product, so a mirrored UV island
            // gets its handedness from the UVs and not from a guess.
            // Smoother still: the mesh's own tangent frame, interpolated
            // across the triangle, where it has one. The triangle's frame
            // above is flat across each face, so a normal-mapped curve shows
            // its tessellation; the interpolated one does not.
            let frame = match scene.tangent_frame(hit) {
                Some((along_u, sign)) => match (along_u - n * n.dot(along_u)).try_normalize() {
                    Some(t) => TangentFrame {
                        normal: n,
                        tangent: t,
                        bitangent: n.cross(t) * sign,
                    },
                    None => TangentFrame { normal: n, tangent, bitangent: -down },
                },
                None => TangentFrame {
                    normal: n,
                    tangent,
                    bitangent: -down,
                },
            };
            let sample = main.value(*register).as_color();
            frame.decode_normal_map(vec3(sample.r, sample.g, sample.b), 1.0)
        }
        BumpSource::Height { program, register } => {
            let tangent = derivatives
                .and_then(|(dpdu, _)| (dpdu - n * n.dot(dpdu)).try_normalize())
                .unwrap_or_else(|| TangentFrame::from_normal(n).tangent);
            let bitangent = n.cross(tangent);
            let transform = &scene.instance(hit.instance).transform;
            // A tenth of a footprint in world units, floored so a sharp view
            // still has something to difference across.
            let step = (point.footprint * point.scale * BUMP_FILTER_WIDTH).max(1e-6);

            // The height a world-space offset along the surface away.
            let mut height = |offset: Vec3| -> f32 {
                // The UVs move with the point, so a height map sampled by UV
                // and a pattern sampled by position see the same step.
                let (du, dv) = match derivatives {
                    Some((dpdu, dpdv)) => uv_offset(dpdu, dpdv, offset),
                    None => (0.0, 0.0),
                };
                let moved = SurfacePoint {
                    position: point.position + transform.inv_vector(offset),
                    uv: point.uv + vec2(du, dv),
                    ..*point
                };
                shade(program, &scene.textures, &moved, registers, scratch);
                registers.value(*register).as_float()
            };
            // Central differences, for the slope *at* the point.
            let along_t = (height(tangent * step) - height(tangent * -step)) / (2.0 * step);
            let along_b = (height(bitangent * step) - height(bitangent * -step)) / (2.0 * step);
            // Tilted away from uphill.
            (n - tangent * along_t - bitangent * along_b)
                .try_normalize()
                .unwrap_or(n)
        }
    };
    if bumped.dot(geometric_normal) <= 1e-4 {
        n
    } else {
        bumped
    }
}

/// The change in UV that moves a point by `offset` along the surface, from
/// the surface's UV derivatives: the least-squares solution of
/// `dpdu * du + dpdv * dv = offset`.
fn uv_offset(dpdu: Vec3, dpdv: Vec3, offset: Vec3) -> (f32, f32) {
    let (a, b, c) = (dpdu.dot(dpdu), dpdu.dot(dpdv), dpdv.dot(dpdv));
    let det = a * c - b * b;
    if det.abs() < 1e-20 {
        return (0.0, 0.0);
    }
    let (pu, pv) = (dpdu.dot(offset), dpdv.dot(offset));
    ((c * pu - b * pv) / det, (a * pv - b * pu) / det)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::Environment;
    use crate::scene::{Instance, Mesh};
    use tequila_core::ids::{InstanceId, MaterialId, MeshId};
    use tequila_core::spectrum::Spectrum;
    use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, TextureId};
    use tequila_materials::material_type::MaterialType;
    use tequila_texture::image::{ColorSpace, Image};
    use tequila_texture::projection::Mapping;
    use tequila_texture::texture::Texture;

    /// A unit quad in the XY plane facing +Z, with UVs as glTF lays them out:
    /// u along +x, v running *down* — v = 0 at the top edge, y = 1.
    fn quad() -> Mesh {
        Mesh::new(
            vec![
                vec3(0.0, 0.0, 0.0),
                vec3(1.0, 0.0, 0.0),
                vec3(1.0, 1.0, 0.0),
                vec3(0.0, 1.0, 0.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
        .with_uvs(vec![vec2(0.0, 1.0), vec2(1.0, 1.0), vec2(1.0, 0.0), vec2(0.0, 0.0)])
    }

    fn texture(width: usize, role: MapRole, texel: impl Fn(usize) -> [f32; 3]) -> Arc<Texture> {
        let mut data = Vec::with_capacity(width * width * 3);
        for y in 0..width {
            for x in 0..width {
                let _ = y;
                data.extend_from_slice(&texel(x));
            }
        }
        let image = Image {
            width,
            height: width,
            channels: 3,
            data,
            color_space: ColorSpace::Raw,
        };
        Arc::new(Texture::from_linear(image, role, Mapping::default()))
    }

    /// A scene of one quad with a material whose Bump is `wire`d by the
    /// caller, and the texture the graph samples.
    fn scene_with(texture: Arc<Texture>, wire: impl Fn(&mut MaterialGraph, NodeId)) -> SceneSnapshot {
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let image = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(0),
            fallback: tequila_materials::graph::Value::Float(0.0),
        }));
        wire(&mut graph, image);
        let program = graph.compile().expect("compiles");
        let mut table = TextureTable::new();
        table.insert(TextureId(0), texture);
        SceneSnapshot::new(
            vec![Arc::new(quad())],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
            vec![Material::diffuse(Spectrum::splat(0.5))],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        )
        .with_programs(vec![Some(Arc::new(program))], Arc::new(table))
    }
    use tequila_materials::graph::NodeId;

    /// The normal at the middle of the quad, with the bump applied.
    fn bumped_at_centre(scene: &SceneSnapshot) -> Vec3 {
        let hit = Hit {
            t: 1.0,
            position: vec3(0.5, 0.5, 0.0),
            geometric_normal: Vec3::Z,
            shading_normal: Vec3::Z,
            uv: vec2(0.5, 0.5),
            instance: InstanceId(0),
            material: MaterialId(0),
            primitive: 0,
        };
        let point = scene.shading_point(&hit, Vec3::Z);
        let program = scene.program(MaterialId(0)).expect("a program");
        let mut main = Registers::new();
        let mut material = Material::default();
        shade(program, &scene.textures, &point, &mut main, &mut material);
        let source = scene.bump(MaterialId(0)).expect("the material bumps");
        bumped_normal(
            source,
            scene,
            &hit,
            &point,
            Vec3::Z,
            Vec3::Z,
            &main,
            &mut Registers::new(),
            &mut Material::default(),
        )
    }

    /// A normal map laid down by a projection leans along the projection,
    /// not along the mesh's UVs. Here the UVs run turned a quarter from the
    /// world (u up +y), and the tri-planar projection lays the image across
    /// +x on this face: a texel leaning along the image's width must lean
    /// the surface towards +x. Decoded in the UV frame, as texture packs'
    /// normal maps were, it leaned towards +y — every bump on a tri-planar
    /// part pointing a quarter turn away from where its picture said.
    #[test]
    fn a_projected_normal_map_leans_along_its_projection() {
        let right = texture(4, MapRole::Normal, |_| [0.8, 0.5, 0.9]);
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping {
                projection: tequila_texture::projection::Projection::TriPlanar,
                ..Mapping::default()
            },
        }));
        let image = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(0),
            fallback: tequila_materials::graph::Value::Float(0.0),
        }));
        graph.connect(mapping, image, Socket::Input(0));
        let output = graph.output();
        graph.connect(image, output, Socket::Param(Param::Bump));
        let program = graph.compile().expect("compiles");
        let mut table = TextureTable::new();
        table.insert(TextureId(0), right);
        let turned = Mesh::new(
            vec![vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2, 0, 2, 3],
        )
        .with_uvs(vec![vec2(0.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0)]);
        let scene = SceneSnapshot::new(
            vec![Arc::new(turned)],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
            vec![Material::diffuse(Spectrum::splat(0.5))],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        )
        .with_programs(vec![Some(Arc::new(program))], Arc::new(table));
        let n = bumped_at_centre(&scene);
        assert!(n.x > 0.3 && n.y.abs() < 1e-2, "leaned along the mesh's UVs, not the projection: {n:?}");
    }

    /// glTF's convention, pinned: a normal map texel leaning "up" the image
    /// (green above half) leans the surface towards the top of the texture —
    /// here +y, because v runs down it. The first guess at this is wrong half
    /// the time, and wrong looks like light coming from below.
    #[test]
    fn a_normal_map_leans_the_way_gltf_means() {
        let up = texture(4, MapRole::Normal, |_| [0.5, 0.8, 0.9]);
        let scene = scene_with(up, |graph, image| {
            let output = graph.output();
            graph.connect(image, output, Socket::Param(Param::Bump));
        });
        assert!(matches!(scene.bump(MaterialId(0)), Some(BumpSource::NormalMap { .. })));
        let n = bumped_at_centre(&scene);
        assert!(n.y > 0.3, "leaning up the image should lean towards +y: {n:?}");
        assert!(n.x.abs() < 1e-3, "and not sideways: {n:?}");

        // And red leans along u, which is +x.
        let right = texture(4, MapRole::Normal, |_| [0.8, 0.5, 0.9]);
        let scene = scene_with(right, |graph, image| {
            let output = graph.output();
            graph.connect(image, output, Socket::Param(Param::Bump));
        });
        let n = bumped_at_centre(&scene);
        assert!(n.x > 0.3 && n.y.abs() < 1e-3, "{n:?}");
    }

    /// With the mesh's own tangents the normal map decodes against them:
    /// glTF-handed tangents give the same lean as the triangle's frame, and
    /// a mirrored island's tangents (handedness flipped) lean it the other
    /// way — which the triangle's frame, not knowing about the mirror in the
    /// artwork, could never do.
    #[test]
    fn a_normal_map_uses_the_meshs_tangents() {
        let lean = |w: f32| {
            let up = texture(4, MapRole::Normal, |_| [0.5, 0.8, 0.9]);
            let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
            let image = graph.add_node(Node::new(NodeKind::ImageTexture {
                texture: TextureId(0),
                fallback: tequila_materials::graph::Value::Float(0.0),
            }));
            let output = graph.output();
            graph.connect(image, output, Socket::Param(Param::Bump));
            let program = graph.compile().expect("compiles");
            let mut table = TextureTable::new();
            table.insert(TextureId(0), up);
            let mesh = quad().with_tangents(vec![tequila_core::math::Vec4 { x: 1.0, y: 0.0, z: 0.0, w }; 4]);
            let scene = SceneSnapshot::new(
                vec![Arc::new(mesh)],
                vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
                vec![Material::diffuse(Spectrum::splat(0.5))],
                Arc::new(Environment::uniform(Spectrum::ONE)),
            )
            .with_programs(vec![Some(Arc::new(program))], Arc::new(table));
            bumped_at_centre(&scene)
        };
        let handed = lean(1.0);
        assert!(handed.y > 0.3 && handed.x.abs() < 1e-3, "{handed:?}");
        let mirrored = lean(-1.0);
        assert!(mirrored.y < -0.3, "a mirrored island leaned {mirrored:?}");
    }

    /// A height rising along u through a Bump node tilts the normal away from
    /// uphill by exactly the slope: depth over the quad's width.
    #[test]
    fn a_height_ramp_tilts_by_its_slope() {
        let size = 64;
        // Rising left to right, 0 to 1, as a height map (data, not colour).
        let ramp = texture(size, MapRole::Data, |x| {
            let h = x as f32 / (size - 1) as f32;
            [h, h, h]
        });
        let depth = 0.1;
        let scene = scene_with(ramp, |graph, image| {
            let bump = graph.add_node(Node::new(NodeKind::Bump { depth }));
            graph.connect(image, bump, Socket::Input(0));
            let output = graph.output();
            graph.connect(bump, output, Socket::Param(Param::Bump));
        });
        assert!(matches!(scene.bump(MaterialId(0)), Some(BumpSource::Height { .. })));
        let n = bumped_at_centre(&scene);
        // The texture spans the quad's unit width in (size - 1)/size of its
        // texels' centres, so the slope is depth over that.
        let slope = depth * (size - 1) as f32 / size as f32;
        let expected = vec3(-slope, 0.0, 1.0).normalize();
        assert!((n - expected).length() < 0.01, "got {n:?}, expected {expected:?}");
    }

    /// Nothing wired into Bump, nothing to do.
    #[test]
    fn an_unbumped_material_has_no_source() {
        let flat = texture(2, MapRole::Color, |_| [0.5, 0.5, 0.5]);
        let scene = scene_with(flat, |graph, image| {
            let output = graph.output();
            graph.connect(image, output, Socket::Param(Param::BaseColor));
        });
        assert!(scene.bump(MaterialId(0)).is_none());
    }
}
