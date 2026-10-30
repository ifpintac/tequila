//! Running a compiled material graph at a shading point
//! (`docs/02-MATERIAL-SYSTEM.md` §4.3).
//!
//! The compiler turns a graph into a flat list of instructions over a small
//! bank of registers. This executes that list: one pass, in order, no branching
//! and no allocation, producing the §2.2 parameter block the BSDF is built
//! from.
//!
//! ## Why there is a fast path, and why it is not an optimisation
//!
//! A material with no textures compiles to an **empty** instruction list, and
//! its parameters are already the constants the compiler folded. Running an
//! empty program and copying a block that did not change would cost a copy of a
//! 35-parameter struct at every path vertex on every core — for nothing.
//! [`shade`] returns the constants by reference in that case, so a material
//! that does not use the machinery does not pay for it.
//!
//! That matters more than it sounds. Most materials in most scenes have no
//! textures at all: fasteners, internal brackets, the inside of a housing.
//! Making them pay for the general case is how a feature that helps a few
//! surfaces slows down the whole render.

use crate::material::Material;
use tequila_materials::graph::{CompiledGraph, Instruction, TextureId, Value};
use tequila_materials::material_type::{Param, ParamUi, ParamValue};
use tequila_materials::nodes::PointInput;
use tequila_texture::projection::SurfacePoint;
use tequila_texture::texture::Texture;
use std::sync::Arc;

/// Textures a program can sample, indexed by the ids its instructions carry.
#[derive(Clone, Debug, Default)]
pub struct TextureTable {
    entries: Vec<Option<Arc<Texture>>>,
    /// Volume Maps' density grids, by the same ids: a VDB is a texture-store
    /// entry of the Volume role, loaded as a grid rather than an image.
    volumes: Vec<Option<Arc<tequila_texture::volume::VolumeGrid>>>,
}

impl TextureTable {
    pub fn new() -> TextureTable {
        TextureTable {
            entries: Vec::new(),
            volumes: Vec::new(),
        }
    }

    pub fn insert_volume(&mut self, id: TextureId, grid: Arc<tequila_texture::volume::VolumeGrid>) {
        let index = id.0 as usize;
        if self.volumes.len() <= index {
            self.volumes.resize(index + 1, None);
        }
        self.volumes[index] = Some(grid);
    }

    pub fn volume(&self, id: TextureId) -> Option<&Arc<tequila_texture::volume::VolumeGrid>> {
        self.volumes.get(id.0 as usize).and_then(|e| e.as_ref())
    }

    pub fn insert(&mut self, id: TextureId, texture: Arc<Texture>) {
        let index = id.0 as usize;
        if self.entries.len() <= index {
            self.entries.resize(index + 1, None);
        }
        self.entries[index] = Some(texture);
    }

    pub fn get(&self, id: TextureId) -> Option<&Arc<Texture>> {
        self.entries.get(id.0 as usize).and_then(|e| e.as_ref())
    }

    pub fn len(&self) -> usize {
        self.entries.iter().filter(|e| e.is_some()).count()
    }

    /// Whether `other` holds the same pictures and grids under the same ids:
    /// the same allocations, as a snapshot rebuilt around unchanged files
    /// has them, so nothing is compared pixel by pixel.
    pub fn same_as(&self, other: &TextureTable) -> bool {
        fn same<T>(a: &[Option<Arc<T>>], b: &[Option<Arc<T>>]) -> bool {
            (0..a.len().max(b.len())).all(|i| match (a.get(i).and_then(|e| e.as_ref()), b.get(i).and_then(|e| e.as_ref())) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            })
        }
        same(&self.entries, &other.entries) && same(&self.volumes, &other.volumes)
    }

    /// Whether `id` names the same picture in both, or nothing in both.
    pub fn same_entry(&self, other: &TextureTable, id: TextureId) -> bool {
        match (self.get(id), other.get(id)) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Registers for one evaluation.
///
/// Sized once per thread and reused, so shading allocates nothing. Sixteen
/// covers every graph the node catalog can currently express; a deeper one
/// grows the vector once and then stops.
#[derive(Clone, Debug, Default)]
pub struct Registers {
    slots: Vec<Value>,
}

impl Registers {
    pub fn new() -> Registers {
        Registers {
            slots: Vec::with_capacity(16),
        }
    }

    /// What a register held after the last evaluation.
    pub fn value(&self, register: tequila_materials::graph::Register) -> Value {
        self.slots
            .get(register.0 as usize)
            .copied()
            .unwrap_or(Value::Float(0.0))
    }

    fn prepare(&mut self, count: usize) {
        self.slots.clear();
        self.slots.resize(count.max(1), Value::Float(0.0));
    }
}

/// More inputs than any node has.
const MAX_ARGS: usize = 24;

/// What a shading point says to a node that reads it - Blender's Texture
/// Coordinate, Geometry and Object Info, as Cycles defines each.
pub fn point_input(input: PointInput, point: &SurfacePoint) -> Value {
    use tequila_core::math::vec3;
    let world = &point.world;
    match input {
        // In scene units, whatever the part's own scale: Tequila's
        // patterns' space, so a pattern's size means the same on every part.
        PointInput::Object => Value::Vector(point.position * point.scale),
        PointInput::Generated => Value::Vector(world.generated),
        // Blender's v runs up the image; Tequila's, as glTF's, down it.
        PointInput::Uv => Value::Vector(vec3(point.uv.x, 1.0 - point.uv.y, 0.0)),
        PointInput::ObjectNormal => Value::Vector(point.normal),
        PointInput::Reflection => {
            let (n, i) = (world.normal, world.incoming);
            Value::Vector(n * (2.0 * n.dot(i)) - i)
        }
        PointInput::Position => Value::Vector(world.position),
        PointInput::Normal => Value::Vector(world.normal),
        PointInput::TrueNormal => Value::Vector(world.true_normal),
        PointInput::Incoming => Value::Vector(world.incoming),
        PointInput::Backfacing => Value::Float(if world.backfacing { 1.0 } else { 0.0 }),
        PointInput::ObjectLocation => Value::Vector(world.object_location),
        PointInput::ObjectRandom => Value::Float(world.object_random),
    }
}

/// Evaluate a material at a shading point.
///
/// Returns the constants directly when the program is empty, which is the
/// common case — see this module's header.
pub fn shade<'a>(
    program: &'a CompiledGraph,
    textures: &TextureTable,
    point: &SurfacePoint,
    registers: &mut Registers,
    scratch: &'a mut Material,
) -> &'a Material {
    if (program.instructions.is_empty() || program.driven.is_empty()) && program.decals.is_empty() {
        return &program.constants;
    }

    registers.prepare(program.registers);
    for instruction in &program.instructions {
        let value = match instruction {
            Instruction::Load { value, .. } => *value,
            Instruction::Project { mapping, .. } => {
                // The coordinates a texture lookup will use, carried as a
                // vector so the footprint travels with them: `z` holds the
                // footprint, which the sample instruction needs to pick a MIP
                // level and which would otherwise have to be recomputed.
                let contributions = mapping.project(point);
                match contributions.first() {
                    Some(first) => Value::Vector(tequila_core::math::vec3(
                        first.coords.u,
                        first.coords.v,
                        first.coords.width(),
                    )),
                    None => Value::Vector(tequila_core::math::Vec3::ZERO),
                }
            }
            Instruction::Evaluate {
                procedural,
                seed,
                mapping,
                ..
            } => {
                // Object space by default (§6), so a procedural does not swim
                // across a part as the part moves. A mapping node moves it
                // into whatever space that node describes, by projecting the
                // shading point through it and reading the result back as a
                // position.
                let mapped = mapping
                    .as_ref()
                    .and_then(|mapping| mapping.project(point).first().map(|first| first.coords));
                Value::Float(match mapped {
                    // A flat pattern lies on the mapping's (u, v); a solid one
                    // is sampled on that plane of its space.
                    Some(coords) => procedural.evaluate_uv(coords.u, coords.v, *seed),
                    // In object space, measured in scene units.
                    None => procedural.evaluate(point.position * point.scale, *seed),
                })
            }
            Instruction::Curvature {
                mode,
                radius,
                contrast,
                ..
            } => Value::Float(mode.mask(point.curvature, *radius, *contrast)),
            Instruction::VertexColor { .. } => Value::Color(point.vertex_color),
            Instruction::VolumeMap { .. } => Value::Float(1.0),
            Instruction::Occlusion { contrast, .. } => {
                Value::Float(tequila_materials::graph::occlusion_shape(point.occlusion, *contrast))
            }
            Instruction::Sample {
                texture,
                mapping,
                fallback,
                ..
            } => match textures.get(*texture) {
                // The graph's mapping, not the texture's own. Projecting
                // through the texture's would discard what the graph asked for
                // and read from somewhere else entirely.
                Some(loaded) => Value::Color(colour_of(loaded, loaded.sample_with(mapping, point))),
                // A texture that has not loaded yet, or that failed to: the
                // material keeps the value it had rather than turning black
                // (§5.1's placeholder rule).
                None => *fallback,
            },
            Instruction::SampleAlpha {
                texture,
                mapping,
                fallback,
                ..
            } => match textures.get(*texture) {
                Some(loaded) => Value::Float(loaded.sample_alpha(mapping, point)),
                None => *fallback,
            },
            Instruction::Apply { kind, args, output, .. } => {
                // On the stack: this runs at every path vertex, and no node
                // has anywhere near this many inputs.
                let mut inputs = [Value::Float(0.0); MAX_ARGS];
                let count = args.len().min(MAX_ARGS);
                for (slot, r) in inputs.iter_mut().zip(&args[..count]) {
                    *slot = registers.slots[r.0 as usize];
                }
                kind.evaluate_output(&inputs[..count], *output)
            }
            Instruction::Input { input, .. } => point_input(*input, point),
        };
        let destination = instruction.destination().0 as usize;
        if destination < registers.slots.len() {
            registers.slots[destination] = value;
        }
    }

    *scratch = program.constants.clone();
    for (param, register) in &program.driven {
        let Some(value) = registers.slots.get(register.0 as usize) else {
            continue;
        };
        write_param(scratch, *param, *value);
    }
    for decal in &program.decals {
        if !decal.enabled || decal.opacity <= 0.0 || !decal.prints_on(point.normal) { continue; }
        let Some(texture) = textures.get(decal.texture) else { continue; };
        let (color, coverage) = texture.sample_decal(&decal.mapping, point);
        let color = colour_of(texture, color) * decal.ink;
        let alpha = coverage * decal.opacity;
        scratch.base_color = scratch.base_color * (1.0 - alpha) + color * decal.opacity;
        // Foil is metal where it covers; ink leaves none.
        scratch.base_metalness = scratch.base_metalness * (1.0 - alpha) + if decal.metallic { alpha } else { 0.0 };
        scratch.transmission_weight *= 1.0 - alpha;
        scratch.specular_roughness = scratch.specular_roughness * (1.0 - alpha) + decal.roughness * alpha;
    }
    scratch
}

/// The labels alone at a point, over nothing (`docs/01` §12's Labels pass):
/// every enabled label composited bottom to top as `shade` composites them
/// over the surface, but over transparency, as colour premultiplied by its
/// coverage and the coverage.
pub fn labels_at(
    program: &CompiledGraph,
    textures: &TextureTable,
    point: &SurfacePoint,
) -> (tequila_core::spectrum::Spectrum, f32) {
    let mut colour = tequila_core::spectrum::Spectrum::ZERO;
    let mut coverage = 0.0f32;
    for decal in &program.decals {
        if !decal.enabled || decal.opacity <= 0.0 || !decal.prints_on(point.normal) {
            continue;
        }
        let Some(texture) = textures.get(decal.texture) else { continue };
        let (color, covered) = texture.sample_decal(&decal.mapping, point);
        let color = colour_of(texture, color) * decal.ink;
        let alpha = covered * decal.opacity;
        colour = colour * (1.0 - alpha) + color * decal.opacity;
        coverage = coverage * (1.0 - alpha) + alpha;
    }
    (colour, coverage)
}

/// A texel as the renderer works with it.
///
/// A colour picture's texels are linear sRGB once their transfer curve is
/// off — Rec.709 primaries — and the renderer works in ACEScg, whose
/// primaries are wider. Used as they stood, every picture rendered more
/// saturated than it is: a faint beige stain on polystyrene came out orange,
/// a near-grey bluish, and a texture of a colour did not match the same
/// colour picked in the Material tab, which the picker converts. The Solid
/// view converted all along, so the two views disagreed about every texture.
/// Numbers (roughness, normals, masks) are not colours and pass unchanged.
fn colour_of(texture: &tequila_texture::texture::Texture, v: tequila_core::math::Vec3) -> tequila_core::spectrum::Spectrum {
    let colour_managed = texture.role == tequila_texture::texture::MapRole::Color
        && texture.source_color_space != tequila_texture::image::ColorSpace::Raw;
    if colour_managed {
        tequila_core::spectrum::Spectrum::from_linear_srgb(v)
    } else {
        spectrum_of(v)
    }
}

fn spectrum_of(v: tequila_core::math::Vec3) -> tequila_core::spectrum::Spectrum {
    tequila_core::spectrum::Spectrum {
        r: v.x,
        g: v.y,
        b: v.z,
    }
}

fn write_param(material: &mut Material, param: Param, value: Value) {
    let written = match param.ui() {
        ParamUi::Color | ParamUi::Vector => ParamValue::Color(value.as_color()),
        ParamUi::Flag => ParamValue::Flag(value.as_float() > 0.5),
        ParamUi::Scalar => ParamValue::Scalar(value.as_float()),
        ParamUi::Slot | ParamUi::Metal => return,
    };
    param.write(material, written);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3, Vec3};
    use tequila_core::spectrum::{spectrum, Spectrum};
    use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket};
    use tequila_materials::material_type::MaterialType;
    use tequila_texture::image::{ColorSpace, Image};
    use tequila_texture::projection::{Mapping, Projection};
    use tequila_texture::texture::MapRole;

    fn point_at(position: Vec3) -> SurfacePoint {
        SurfacePoint {
            position,
            normal: vec3(0.0, 1.0, 0.0),
            uv: vec2(0.25, 0.75),
            footprint: 0.0,
            scale: 2.0,
            uv_footprint: None,
            curvature: 0.0,
            occlusion: 0.0,
            vertex_color: Spectrum::ONE,
            world: tequila_texture::projection::PointWorld {
                position: position * 2.0 + vec3(1.0, 0.0, 0.0),
                normal: vec3(0.0, 0.6, 0.8),
                true_normal: vec3(0.0, 1.0, 0.0),
                incoming: vec3(0.0, 0.0, 1.0),
                backfacing: false,
                generated: vec3(0.1, 0.2, 0.3),
                object_location: vec3(1.0, 0.0, 0.0),
                object_random: 0.42,
            },
        }
    }

    /// A Blender node shades as it evaluates: a noise's colour - its second
    /// output - read at the point in scene units, its first output on
    /// another channel, the two from one node.
    #[test]
    fn a_blender_node_shades_as_it_evaluates() {
        use tequila_materials::graph::Value;
        use tequila_materials::material_type::Param;
        use tequila_materials::nodes::ShaderNode;
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let noise = graph.add_node(Node::new(NodeKind::Node(ShaderNode::noise())).with_literal(Socket::Input(2), Value::Float(3.0)));
        let output = graph.output();
        graph.connect_output(noise, 1, output, Socket::Param(Param::BaseColor));
        graph.connect_output(noise, 0, output, Socket::Param(Param::SpecularRoughness));
        let program = graph.compile().expect("compiles");
        let textures = TextureTable::new();
        let (mut registers, mut scratch) = (Registers::new(), Material::default());
        for p in [vec3(0.1, 0.2, 0.3), vec3(-0.4, 0.05, 0.7)] {
            let point = point_at(p);
            let shaded = shade(&program, &textures, &point, &mut registers, &mut scratch).clone();
            let mut inputs: Vec<Value> = ShaderNode::noise().inputs().iter().map(|s| s.default).collect();
            // Object space in scene units: the point's position times its scale.
            inputs[0] = Value::Vector(p * 2.0);
            inputs[2] = Value::Float(3.0);
            let colour = ShaderNode::noise().evaluate(&inputs, 1).as_color();
            let fac = ShaderNode::noise().evaluate(&inputs, 0).as_float();
            assert!((shaded.base_color.r - colour.r).abs() < 1e-5 && (shaded.base_color.b - colour.b).abs() < 1e-5, "{:?} != {colour:?}", shaded.base_color);
            assert!((shaded.specular_roughness - fac.clamp(0.0, 1.0)).abs() < 1e-5, "{} != {fac}", shaded.specular_roughness);
        }
    }

    /// What the point says to the nodes that read it, as Cycles defines
    /// each: Blender's v up, the reflection about the shading normal.
    #[test]
    fn point_inputs_read_the_point() {
        use tequila_materials::graph::Value;
        use tequila_materials::nodes::PointInput;
        let point = point_at(vec3(0.5, 0.25, 0.0));
        assert_eq!(point_input(PointInput::Object, &point), Value::Vector(vec3(1.0, 0.5, 0.0)));
        assert_eq!(point_input(PointInput::Uv, &point), Value::Vector(vec3(0.25, 0.25, 0.0)));
        assert_eq!(point_input(PointInput::ObjectRandom, &point), Value::Float(0.42));
        assert_eq!(point_input(PointInput::Backfacing, &point), Value::Float(0.0));
        let Value::Vector(r) = point_input(PointInput::Reflection, &point) else { panic!() };
        // 2(n.i)n - i with n = (0, .6, .8), i = z: (0, .96, .28).
        assert!((r - vec3(0.0, 0.96, 0.28)).length() < 1e-5, "{r:?}");
    }

    /// A procedural on base colour reaches the shaded material, through the
    /// ramp, at the right colours.
    ///
    /// The evaluator is generic over the procedural enum, so a new node is
    /// *supposed* to render with no further work — and "supposed to" is the
    /// claim this project has been wrong about most often. Bump maps, texture
    /// mapping, scene units and vertex snapping were each complete in the
    /// engine and unreachable, and none of the checklists could tell. So the
    /// path is walked rather than reasoned about: compile the graph, shade two
    /// points, and check the colours are the ramp's.
    #[test]
    fn a_wood_procedural_reaches_the_shaded_material() {
        use tequila_texture::gradient::{Gradient, Stop};
        use tequila_texture::noise::Fractal;
        use tequila_texture::procedural::{Axis, Procedural};

        // Rings only: no wander, no grain, so the field is `frac(radius * 4)`
        // and the value at a chosen radius is known exactly.
        let wood = Procedural::Wood {
            axis: Axis::Y,
            frequency: 4.0,
            distortion: 0.0,
            grain: 0.0,
            fractal: Fractal::default(),
        };
        // A ramp with a hard step at the half-ring, so the two sides are two
        // unmistakable colours rather than two shades of one.
        let ramp = Gradient::new(vec![
            Stop {
                at: 0.0,
                color: spectrum(1.0, 0.0, 0.0),
            },
            Stop {
                at: 0.5,
                color: spectrum(1.0, 0.0, 0.0),
            },
            Stop {
                at: 0.5,
                color: spectrum(0.0, 0.0, 1.0),
            },
            Stop {
                at: 1.0,
                color: spectrum(0.0, 0.0, 1.0),
            },
        ]);

        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let node = graph.add_node(Node::new(NodeKind::Procedural {
            procedural: Box::new(wood),
            seed: 11,
        }));
        let gradient = graph.add_node(Node::new(NodeKind::Gradient {
            gradient: Box::new(ramp),
        }));
        let output = graph.output();
        graph.connect(node, gradient, Socket::Input(0));
        graph.connect(
            gradient,
            output,
            Socket::Param(tequila_materials::material_type::Param::BaseColor),
        );

        let program = graph.compile().expect("the graph compiles");
        let textures = TextureTable::new();
        let at = |x: f32, z: f32| {
            let point = SurfacePoint {
                position: vec3(x, 0.3, z),
                normal: vec3(0.0, 1.0, 0.0),
                uv: vec2(0.0, 0.0),
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: tequila_core::spectrum::Spectrum::ONE,
                world: Default::default(),
            };
            shade(
                &program,
                &textures,
                &point,
                &mut Registers::new(),
                &mut Material::default(),
            )
            .base_color
        };

        // radius 0.05 -> frac(0.2) = 0.2, the low half of the ramp: red.
        let inner = at(0.05, 0.0);
        assert!(
            inner.r > 0.9 && inner.b < 0.1,
            "the inner ring shaded {inner:?} rather than red"
        );
        // radius 0.2 -> frac(0.8) = 0.8, the high half: blue.
        let outer = at(0.2, 0.0);
        assert!(
            outer.b > 0.9 && outer.r < 0.1,
            "the outer ring shaded {outer:?} rather than blue"
        );

        // And the rings are concentric once shaded, not only in the
        // procedural: the same radius at another angle is the same colour.
        let sideways = at(0.0, 0.05);
        assert_eq!(
            (inner.r, inner.b),
            (sideways.r, sideways.b),
            "the same radius shaded differently at another angle"
        );
    }

    /// Marble reaches it too, and its bands run across the axis it was given.
    #[test]
    fn a_marble_procedural_bands_the_shaded_material() {
        use tequila_texture::noise::Fractal;
        use tequila_texture::procedural::{Axis, Procedural};

        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let node = graph.add_node(Node::new(NodeKind::Procedural {
            procedural: Box::new(Procedural::Marble {
                axis: Axis::Y,
                frequency: 6.0,
                distortion: 0.0,
                fractal: Fractal::default(),
            }),
            seed: 3,
        }));
        let output = graph.output();
        // Straight onto a scalar, with no ramp — which is the other half of
        // the wiring, and the half a colour-only test would miss.
        graph.connect(
            node,
            output,
            Socket::Param(tequila_materials::material_type::Param::SpecularRoughness),
        );

        let program = graph.compile().expect("the graph compiles");
        let textures = TextureTable::new();
        let at = |y: f32| {
            let point = SurfacePoint {
                position: vec3(0.2, y, 0.1),
                normal: vec3(0.0, 1.0, 0.0),
                uv: vec2(0.0, 0.0),
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: tequila_core::spectrum::Spectrum::ONE,
                world: Default::default(),
            };
            shade(
                &program,
                &textures,
                &point,
                &mut Registers::new(),
                &mut Material::default(),
            )
            .specular_roughness
        };

        // The peak and the trough of the sine, which are a quarter and three
        // quarters of the way through a band: `sin(y * 6)` has period
        // 2*PI/6 = PI/3, so those are y = PI/12 and y = PI/4.
        //
        // Half a period apart — y = 0 and y = PI/6 — was the first attempt and
        // it compares two *zero crossings*, both of which shade to 0.5. That
        // reads exactly like a constant-folded node, which is what it was
        // mistaken for.
        let peak = at(std::f32::consts::PI / 12.0);
        let trough = at(std::f32::consts::PI / 4.0);
        assert!(
            (peak - trough).abs() > 0.5,
            "the peak and trough of a band shaded {peak} and {trough}; the \
             bands are not there"
        );
        // Across the axis, an undistorted marble is constant.
        let reference = at(0.0);
        let across = {
            let point = SurfacePoint {
                position: vec3(0.9, 0.0, 0.7),
                normal: vec3(0.0, 1.0, 0.0),
                uv: vec2(0.0, 0.0),
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: tequila_core::spectrum::Spectrum::ONE,
                world: Default::default(),
            };
            shade(
                &program,
                &textures,
                &point,
                &mut Registers::new(),
                &mut Material::default(),
            )
            .specular_roughness
        };
        assert!(
            (reference - across).abs() < 1e-5,
            "moving across the axis changed the roughness from {reference} to {across}"
        );
    }

    /// A Mapping node wired into a pattern's Coordinates moves it onto the
    /// mapping's (u, v): a flat pattern — scratches here — then varies along
    /// v, and the mapping's scale changes how fine it is.
    #[test]
    fn a_mapping_node_drives_a_pattern() {
        use tequila_texture::procedural::Procedural;
        use tequila_texture::projection::Mapping;

        let scratches = Procedural::catalog()[2];
        assert!(scratches.is_planar());
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping::default(),
        }));
        let pattern = graph.add_node(Node::new(NodeKind::Procedural {
            procedural: Box::new(scratches),
            seed: 5,
        }));
        let output = graph.output();
        graph.connect(mapping, pattern, Socket::Input(0));
        graph.connect(
            pattern,
            output,
            Socket::Param(tequila_materials::material_type::Param::SpecularRoughness),
        );
        let program = graph.compile().expect("a mapping into a pattern compiles");

        let textures = TextureTable::new();
        let at = |u: f32, v: f32| {
            let point = SurfacePoint {
                // Fixed, so only the mapping's (u, v) can move the pattern.
                position: vec3(0.3, 0.2, 0.1),
                normal: vec3(0.0, 1.0, 0.0),
                uv: vec2(u, v),
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: tequila_core::spectrum::Spectrum::ONE,
                world: Default::default(),
            };
            shade(&program, &textures, &point, &mut Registers::new(), &mut Material::default())
                .specular_roughness
        };
        let along_v: Vec<f32> = (0..400).map(|i| at(0.31, i as f32 * 0.003)).collect();
        let low = along_v.iter().copied().fold(f32::MAX, f32::min);
        let high = along_v.iter().copied().fold(f32::MIN, f32::max);
        assert!(high - low > 0.5, "along v the mapped scratches span only {low} to {high}");
        for (i, value) in along_v.iter().enumerate().step_by(37) {
            let expected = scratches.evaluate_uv(0.31, i as f32 * 0.003, 5);
            assert!((value - expected).abs() < 1e-5, "shaded {value}, the pattern says {expected}");
        }
    }

    /// A channel node takes the channel it names, through the whole path.
    ///
    /// Not asserted against `NodeKind::evaluate` — that is the arithmetic, and
    /// the arithmetic was never the risk. The risk is the node folding to a
    /// constant before it runs, which is how the procedural nodes failed: a
    /// node that is not marked varying and whose input is varying must still
    /// survive the folding pass. So this compiles the graph and shades a point
    /// through it.
    #[test]
    fn a_channel_node_picks_its_channel_when_shaded() {
        use crate::shade::{shade, Registers, TextureTable};
        use tequila_core::math::{vec2, vec3};
        use tequila_materials::graph::{
            Channel, MaterialGraph, Node, NodeKind, Socket, TextureId, Value,
        };
        use tequila_materials::material_type::{MaterialType, Param};
        use tequila_texture::image::{ColorSpace, Image};
        use tequila_texture::projection::Mapping;
        use tequila_texture::texture::{MapRole, Texture};
        use std::sync::Arc;

        // A texture whose three channels are three different, known numbers,
        // so "which channel came out" has an unambiguous answer.
        let packed = Arc::new(Texture::from_linear(
            Image {
                width: 2,
                height: 2,
                channels: 3,
                data: vec![0.1, 0.4, 0.9, 0.1, 0.4, 0.9, 0.1, 0.4, 0.9, 0.1, 0.4, 0.9],
                color_space: ColorSpace::Raw,
            },
            MapRole::Data,
            Mapping::default(),
        ));

        for (channel, expected) in [
            (Channel::Red, 0.1f32),
            (Channel::Green, 0.4),
            (Channel::Blue, 0.9),
        ] {
            let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
            let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
                mapping: Mapping::default(),
            }));
            let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
                texture: TextureId(1),
                fallback: Value::Float(0.0),
            }));
            let pick = graph.add_node(Node::new(NodeKind::Channel { channel }));
            let output = graph.output();
            graph.connect(mapping, texture, Socket::Input(0));
            graph.connect(texture, pick, Socket::Input(0));
            graph.connect(pick, output, Socket::Param(Param::SpecularRoughness));

            let program = graph.compile().expect("compiles");
            let mut textures = TextureTable::new();
            textures.insert(TextureId(1), packed.clone());

            let point = tequila_texture::projection::SurfacePoint {
                position: vec3(0.0, 0.0, 0.0),
                normal: vec3(0.0, 1.0, 0.0),
                uv: vec2(0.5, 0.5),
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: tequila_core::spectrum::Spectrum::ONE,
                world: Default::default(),
            };
            let mut scratch = crate::material::Material::default();
            let roughness = shade(
                &program,
                &textures,
                &point,
                &mut Registers::new(),
                &mut scratch,
            )
            .specular_roughness;

            assert!(
                (roughness - expected).abs() < 1e-4,
                "{channel:?} shaded {roughness}, not {expected}; either the \
                 wrong channel came out or the node folded away before it ran"
            );
        }
    }

    /// Two channels of one image drive two parameters independently.
    ///
    /// The arrangement a glTF metallic-roughness map needs: green to
    /// roughness, blue to metalness, one texture sampled once for both.
    #[test]
    fn two_channels_of_one_image_drive_two_parameters() {
        use crate::shade::{shade, Registers, TextureTable};
        use tequila_core::math::{vec2, vec3};
        use tequila_materials::graph::{
            Channel, MaterialGraph, Node, NodeKind, Socket, TextureId, Value,
        };
        use tequila_materials::material_type::{MaterialType, Param};
        use tequila_texture::image::{ColorSpace, Image};
        use tequila_texture::projection::Mapping;
        use tequila_texture::texture::{MapRole, Texture};
        use std::sync::Arc;

        let packed = Arc::new(Texture::from_linear(
            Image {
                width: 1,
                height: 1,
                channels: 3,
                data: vec![0.0, 0.25, 0.75],
                color_space: ColorSpace::Raw,
            },
            MapRole::Data,
            Mapping::default(),
        ));

        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let output = graph.output();
        for (channel, param) in [
            (Channel::Green, Param::SpecularRoughness),
            (Channel::Blue, Param::BaseMetalness),
        ] {
            let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
                mapping: Mapping::default(),
            }));
            let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
                texture: TextureId(1),
                fallback: Value::Float(0.0),
            }));
            let pick = graph.add_node(Node::new(NodeKind::Channel { channel }));
            graph.connect(mapping, texture, Socket::Input(0));
            graph.connect(texture, pick, Socket::Input(0));
            graph.connect(pick, output, Socket::Param(param));
        }

        let program = graph.compile().expect("compiles");
        let mut textures = TextureTable::new();
        textures.insert(TextureId(1), packed);
        let point = tequila_texture::projection::SurfacePoint {
            position: vec3(0.0, 0.0, 0.0),
            normal: vec3(0.0, 1.0, 0.0),
            uv: vec2(0.5, 0.5),
            footprint: 0.0,
            scale: 1.0,
            uv_footprint: None,
            curvature: 0.0,
            occlusion: 0.0,
            vertex_color: tequila_core::spectrum::Spectrum::ONE,
            world: Default::default(),
        };
        let mut scratch = crate::material::Material::default();
        let shaded = shade(
            &program,
            &textures,
            &point,
            &mut Registers::new(),
            &mut scratch,
        );

        assert!(
            (shaded.specular_roughness - 0.25).abs() < 1e-4,
            "roughness took {} rather than the green channel's 0.25",
            shaded.specular_roughness
        );
        assert!(
            (shaded.base_metalness - 0.75).abs() < 1e-4,
            "metalness took {} rather than the blue channel's 0.75",
            shaded.base_metalness
        );
    }

    /// An occlusion map reaches the shading, at full strength and faded.
    ///
    /// glTF's `strength` is a lerp towards *unoccluded*, not a multiply:
    /// `lerp(1, occlusion, strength)`. A multiply gets it backwards — lowering
    /// the strength would darken the model rather than lighten it, and at
    /// strength 0 the surface would be black instead of unshadowed. Both ends
    /// are checked here because both ends are wrong under the multiply and
    /// only the faded one is wrong under a lerp with its arguments swapped.
    #[test]
    fn an_occlusion_map_reaches_the_shading_through_its_red_channel() {
        use crate::shade::{shade, Registers, TextureTable};
        use tequila_core::math::{vec2, vec3};
        use tequila_materials::graph::{
            Channel, MaterialGraph, Node, NodeKind, Socket, TextureId, Value,
        };
        use tequila_materials::material_type::{MaterialType, Param};
        use tequila_texture::image::{ColorSpace, Image};
        use tequila_texture::projection::Mapping;
        use tequila_texture::texture::{MapRole, Texture};
        use std::sync::Arc;

        // ORM: a deep crease in red, and green and blue holding numbers that
        // must not be mistaken for it.
        let orm = Arc::new(Texture::from_linear(
            Image {
                width: 1,
                height: 1,
                channels: 3,
                data: vec![0.25, 0.9, 0.05],
                color_space: ColorSpace::Raw,
            },
            MapRole::Data,
            Mapping::default(),
        ));

        // `None` is the map at full strength; `Some(s)` puts glTF's fade in
        // front of it, as the importer does.
        let occlusion_of = |strength: Option<f32>| {
            let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
            let output = graph.output();
            let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
                mapping: Mapping::default(),
            }));
            let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
                texture: TextureId(1),
                fallback: Value::Float(1.0),
            }));
            let pick = graph.add_node(Node::new(NodeKind::Channel {
                channel: Channel::Red,
            }));
            graph.connect(mapping, texture, Socket::Input(0));
            graph.connect(texture, pick, Socket::Input(0));
            match strength {
                None => graph.connect(pick, output, Socket::Param(Param::Occlusion)),
                Some(s) => {
                    let faded = graph.add_node(Node::new(NodeKind::Remap {
                        from: vec2(0.0, 1.0),
                        to: vec2(1.0 - s, 1.0),
                    }));
                    graph.connect(pick, faded, Socket::Input(0));
                    graph.connect(faded, output, Socket::Param(Param::Occlusion));
                }
            }

            let program = graph.compile().expect("compiles");
            let mut textures = TextureTable::new();
            textures.insert(TextureId(1), orm.clone());
            let point = tequila_texture::projection::SurfacePoint {
                position: vec3(0.0, 0.0, 0.0),
                normal: vec3(0.0, 1.0, 0.0),
                uv: vec2(0.5, 0.5),
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: tequila_core::spectrum::Spectrum::ONE,
                world: Default::default(),
            };
            let mut scratch = tequila_materials::openpbr::Material::default();
            shade(
                &program,
                &textures,
                &point,
                &mut Registers::new(),
                &mut scratch,
            )
            .geometry_occlusion
        };

        let full = occlusion_of(None);
        assert!(
            (full - 0.25).abs() < 1e-4,
            "occlusion shaded {full}, not the red channel's 0.25; either another channel came out or the node folded away before it ran"
        );

        // lerp(1, 0.25, 0.5) = 0.625. A multiply would give 0.125, which is
        // darker than the map at full strength.
        let half = occlusion_of(Some(0.5));
        assert!(
            (half - 0.625).abs() < 1e-4,
            "at strength 0.5 occlusion shaded {half}, not 0.625"
        );

        // And strength 0 means the map is off, not that everything is black.
        let off = occlusion_of(Some(0.0));
        assert!(
            (off - 1.0).abs() < 1e-4,
            "at strength 0 occlusion shaded {off}; the map should be ignored, leaving the surface unoccluded"
        );
    }

    /// Occlusion is not a parameter anyone can see directly — it is a
    /// multiplier on the light a point receives — so it is worth one check
    /// that it darkens the surface rather than merely being stored.
    #[test]
    fn occlusion_darkens_every_lobe_including_a_metal_one() {
        use tequila_core::math::vec3;
        use tequila_materials::material_type::MaterialType;
        use tequila_materials::Bsdf;

        let wo = vec3(0.3, 0.0, 0.954).normalize();
        let wi = vec3(-0.2, 0.1, 0.975).normalize();

        for material_type in [MaterialType::Plastic, MaterialType::Metal] {
            let lit = material_type.defaults();
            let mut shadowed = lit.clone();
            shadowed.geometry_occlusion = 0.25;

            let bright = lit.bsdf(0.0).eval(wo, wi).luminance();
            let dark = shadowed.bsdf(0.0).eval(wo, wi).luminance();
            assert!(bright > 0.0, "{material_type:?} reflects nothing to compare");
            assert!(
                (dark - bright * 0.25).abs() < 1e-5 * bright.max(1.0),
                "{material_type:?} at occlusion 0.25 returned {dark}, not a quarter of its unoccluded {bright}"
            );
        }
    }

    /// A colour picture's texel renders as that colour picked: converted
    /// from the picture's sRGB primaries into the rendering space, as the
    /// colour picker and the Solid view convert. A data picture's numbers
    /// pass as they are. Used unconverted, every texture rendered more
    /// saturated than it is — a beige stain orange, a near-grey bluish.
    #[test]
    fn a_colour_texture_renders_as_that_colour_picked() {
        let beige = vec3(0.60, 0.45, 0.35);
        let picture = |role: MapRole| {
            Arc::new(Texture::from_linear(
                Image { width: 1, height: 1, channels: 3, color_space: ColorSpace::LinearSrgb, data: vec![beige.x, beige.y, beige.z] },
                role,
                Mapping::default(),
            ))
        };
        for (role, expected) in [
            (MapRole::Color, Spectrum::from_linear_srgb(beige)),
            (MapRole::Data, spectrum(beige.x, beige.y, beige.z)),
        ] {
            let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
            let image = graph.add_node(Node::new(NodeKind::ImageTexture {
                texture: TextureId(1),
                fallback: tequila_materials::graph::Value::Color(Spectrum::ZERO),
            }));
            let output = graph.output();
            graph.connect(image, output, Socket::Param(Param::BaseColor));
            let mut textures = TextureTable::new();
            textures.insert(TextureId(1), picture(role));
            let program = graph.compile().unwrap();
            let got = shade(&program, &textures, &point(0.5, 0.5), &mut Registers::new(), &mut Material::default()).base_color;
            assert!(
                (got.r - expected.r).abs() < 1e-5 && (got.g - expected.g).abs() < 1e-5 && (got.b - expected.b).abs() < 1e-5,
                "{role:?}: {got:?}, not {expected:?}"
            );
        }
    }

    #[test]
    fn printed_labels_preserve_alpha_and_layer_order() {
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        graph.material_mut().base_color = spectrum(0.0, 1.0, 0.0);
        graph.decals.push(tequila_materials::decal::Decal::new("Print", TextureId(1)));
        let mut textures = TextureTable::new();
        textures.insert(TextureId(1), Arc::new(Texture::from_linear(Image {
            width: 2, height: 1, channels: 4, color_space: ColorSpace::Raw,
            data: vec![1.0,0.0,0.0,1.0, 0.0,0.0,1.0,0.0],
        }, MapRole::Color, Mapping::default())));
        let evaluate = |graph: &MaterialGraph, p: SurfacePoint| {
            let program = graph.compile().unwrap();
            assert!(program.requirements.textures.contains(&TextureId(1)));
            shade(&program, &textures, &p, &mut Registers::new(), &mut Material::default()).base_color
        };
        let mixed = evaluate(&graph, point(0.5, 0.5));
        assert!((mixed.r - 0.5).abs() < 1e-6 && (mixed.g - 0.5).abs() < 1e-6 && mixed.b == 0.0, "transparent blue must not fringe: {mixed:?}");
        assert_eq!(evaluate(&graph, point(1.25, 0.5)), graph.material().base_color);
        assert_eq!(evaluate(&graph, point(0.75, 0.5)), graph.material().base_color);
        graph.decals[0].opacity = 0.0;
        assert_eq!(evaluate(&graph, point(0.25, 0.5)), graph.material().base_color);
        graph.decals[0].opacity = 1.0;
        let mut top = graph.decals[0].clone();
        top.mapping.offset.x = -0.5;
        graph.decals.push(top);
        assert_eq!(evaluate(&graph, point(0.75, 0.5)), spectrum(1.0, 0.0, 0.0));
        graph.decals[1].enabled = false;
        assert_eq!(evaluate(&graph, point(0.75, 0.5)), graph.material().base_color);

        // Foil is metal where the artwork covers, and ink is none; where the
        // artwork is transparent the surface keeps its own.
        let metalness = |graph: &MaterialGraph, p: SurfacePoint| {
            let program = graph.compile().unwrap();
            shade(&program, &textures, &p, &mut Registers::new(), &mut Material::default()).base_metalness
        };
        assert_eq!(metalness(&graph, point(0.25, 0.5)), 0.0);
        graph.decals[0].metallic = true;
        assert!((metalness(&graph, point(0.25, 0.5)) - 1.0).abs() < 1e-6, "foil is not metal");
        assert_eq!(metalness(&graph, point(0.75, 0.5)), graph.material().base_metalness);
    }

    fn point(u: f32, v: f32) -> SurfacePoint {
        SurfacePoint {
            position: Vec3::ZERO,
            normal: Vec3::Z,
            uv: vec2(u, v),
            footprint: 0.0,
            scale: 1.0,
            uv_footprint: None,
            curvature: 0.0,
            occlusion: 0.0,
            vertex_color: tequila_core::spectrum::Spectrum::ONE,
            world: Default::default(),
        }
    }

    /// A texture that is red on the left half and blue on the right, so a
    /// lookup's position is visible in its result.
    fn split_texture() -> Arc<Texture> {
        let size = 16;
        let mut data = vec![0.0f32; size * size * 3];
        for y in 0..size {
            for x in 0..size {
                let at = (y * size + x) * 3;
                if x < size / 2 {
                    data[at] = 1.0;
                } else {
                    data[at + 2] = 1.0;
                }
            }
        }
        let image = Image {
            width: size,
            height: size,
            channels: 3,
            data,
            color_space: ColorSpace::Raw,
        };
        Arc::new(Texture::from_linear(
            image,
            MapRole::Color,
            Mapping {
                projection: Projection::Uv,
                ..Default::default()
            },
        ))
    }

    fn textured_graph() -> MaterialGraph {
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let output = graph.output();
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping {
                projection: Projection::Uv,
                ..Default::default()
            },
        }));
        let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(1),
            fallback: Value::Color(Spectrum::splat(0.5)),
        }));
        graph.connect(mapping, texture, Socket::Input(0));
        graph.connect(texture, output, Socket::Param(Param::BaseColor));
        graph
    }

    fn table() -> TextureTable {
        let mut table = TextureTable::new();
        table.insert(TextureId(1), split_texture());
        table
    }

    /// The point of the whole module: the texture reaches the material, and
    /// where you look changes what you get.
    #[test]
    fn a_texture_drives_the_parameter_it_is_connected_to() {
        let program = textured_graph().compile().expect("compiles");
        let textures = table();
        let mut registers = Registers::new();
        let mut scratch = Material::default();

        let left = shade(&program, &textures, &point(0.25, 0.5), &mut registers, &mut scratch)
            .base_color;
        assert!(
            left.r > 0.9 && left.b < 0.1,
            "the left half should be red, got {left:?}"
        );

        let mut scratch = Material::default();
        let right = shade(&program, &textures, &point(0.75, 0.5), &mut registers, &mut scratch)
            .base_color;
        assert!(
            right.b > 0.9 && right.r < 0.1,
            "the right half should be blue, got {right:?}"
        );
    }

    /// A material with no textures must not pay for the machinery: the program
    /// is empty and the constants are returned as they are.
    #[test]
    fn an_untextured_material_costs_nothing() {
        let graph = MaterialGraph::from_material(MaterialType::Metal.defaults());
        let program = graph.compile().expect("compiles");
        assert!(program.instructions.is_empty());

        let textures = TextureTable::new();
        let mut registers = Registers::new();
        let mut scratch = Material::default();
        let shaded = shade(&program, &textures, &point(0.5, 0.5), &mut registers, &mut scratch);

        assert_eq!(shaded, &program.constants);
        // The scratch buffer was never written, which is the observable form of
        // "no copy happened".
        assert_eq!(scratch, Material::default());
    }

    /// §5.1's placeholder rule: a texture that has not loaded leaves the
    /// material as it was rather than turning it black, so a scene that is
    /// still reading from disk looks unfinished rather than broken.
    #[test]
    fn a_missing_texture_falls_back_rather_than_going_black() {
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let output = graph.output();
        let node = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(9),
            fallback: Value::Color(spectrum(0.2, 0.4, 0.8)),
        }));
        graph.connect(node, output, Socket::Param(Param::BaseColor));

        let program = graph.compile().expect("compiles");
        let mut registers = Registers::new();
        let mut scratch = Material::default();
        let shaded = shade(
            &program,
            &TextureTable::new(),
            &point(0.5, 0.5),
            &mut registers,
            &mut scratch,
        );

        let colour = shaded.base_color;
        assert!(
            (colour.r - 0.2).abs() < 1e-4 && (colour.b - 0.8).abs() < 1e-4,
            "a missing texture gave {colour:?} rather than its fallback"
        );
    }

    /// A chain of nodes downstream of a texture is executed in order, and the
    /// result is what running them by hand would give.
    #[test]
    fn a_chain_of_nodes_runs_in_order() {
        let mut graph = textured_graph();
        let output = graph.output();
        let texture = graph
            .nodes()
            .find(|(_, n)| matches!(n.kind, NodeKind::ImageTexture { .. }))
            .map(|(id, _)| id)
            .expect("the texture node");
        let invert = graph.add_node(Node::new(NodeKind::Invert));
        graph.connect(texture, invert, Socket::Input(0));
        graph.connect(invert, output, Socket::Param(Param::BaseColor));

        let program = graph.compile().expect("compiles");
        let mut registers = Registers::new();
        let mut scratch = Material::default();
        // The left half is red (1, 0, 0); inverted it is cyan (0, 1, 1).
        let colour = shade(&program, &table(), &point(0.25, 0.5), &mut registers, &mut scratch)
            .base_color;

        assert!(
            colour.r < 0.1 && colour.g > 0.9 && colour.b > 0.9,
            "inverting red should give cyan, got {colour:?}"
        );
    }

    /// Several parameters can be driven at once, and each gets its own value.
    #[test]
    fn several_parameters_can_be_driven_independently() {
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let output = graph.output();

        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping::default(),
        }));
        let colour = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(1),
            fallback: Value::Color(Spectrum::splat(0.5)),
        }));
        let rough = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(1),
            fallback: Value::Color(Spectrum::splat(0.5)),
        }));
        graph.connect(mapping, colour, Socket::Input(0));
        graph.connect(mapping, rough, Socket::Input(0));
        graph.connect(colour, output, Socket::Param(Param::BaseColor));
        graph.connect(rough, output, Socket::Param(Param::SpecularRoughness));

        let program = graph.compile().expect("compiles");
        assert_eq!(program.driven.len(), 2);

        let mut registers = Registers::new();
        let mut scratch = Material::default();
        let shaded = shade(&program, &table(), &point(0.25, 0.5), &mut registers, &mut scratch);

        assert!(shaded.base_color.r > 0.9, "colour: {:?}", shaded.base_color);
        // Red's luminance drives the roughness, which is neither 0 nor 1.
        assert!(
            shaded.specular_roughness > 0.0 && shaded.specular_roughness < 1.0,
            "roughness: {}",
            shaded.specular_roughness
        );
    }

    /// Shading must not allocate per call once the registers are warm, and must
    /// give the same answer every time.
    #[test]
    fn shading_is_repeatable_and_reuses_its_registers() {
        let program = textured_graph().compile().expect("compiles");
        let textures = table();
        let mut registers = Registers::new();

        let mut first = Material::default();
        let a = shade(&program, &textures, &point(0.3, 0.7), &mut registers, &mut first).clone();
        let capacity = registers.slots.capacity();

        for _ in 0..100 {
            let mut scratch = Material::default();
            let b = shade(&program, &textures, &point(0.3, 0.7), &mut registers, &mut scratch);
            assert_eq!(&a, b, "shading the same point twice gave different answers");
        }
        assert_eq!(
            registers.slots.capacity(),
            capacity,
            "the register bank grew during shading"
        );
    }

    /// Tri-planar shading needs no UVs, which is the case CAD geometry is in.
    #[test]
    fn a_tri_planar_material_shades_without_uvs() {
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let output = graph.output();
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping {
                projection: Projection::TriPlanar,
                ..Default::default()
            },
        }));
        let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(1),
            fallback: Value::Color(Spectrum::splat(0.5)),
        }));
        graph.connect(mapping, texture, Socket::Input(0));
        graph.connect(texture, output, Socket::Param(Param::BaseColor));

        let program = graph.compile().expect("compiles");
        assert!(program.requirements.uv_sets.is_empty());

        let mut registers = Registers::new();
        let mut scratch = Material::default();
        let mut at = point(0.0, 0.0);
        at.position = vec3(0.3, 0.4, 0.2);
        at.normal = Vec3::Y;
        let shaded = shade(&program, &table(), &at, &mut registers, &mut scratch);
        assert!(
            shaded.base_color.is_finite(),
            "tri-planar shading produced {:?}",
            shaded.base_color
        );
    }

    #[test]
    fn the_texture_table_is_sparse_and_tolerant() {
        let mut table = TextureTable::new();
        assert!(table.is_empty());
        assert!(table.get(TextureId(7)).is_none());

        table.insert(TextureId(7), split_texture());
        assert_eq!(table.len(), 1);
        assert!(table.get(TextureId(7)).is_some());
        assert!(table.get(TextureId(0)).is_none());
        assert!(table.get(TextureId(9999)).is_none());
    }
}
