//! Tequila's FDM Layer Lines against the Blender group it is ported from,
//! output by output, at the corners of a sphere Blender baked
//! (`scripts/fdm/bake_reference.py`):
//!
//!     TEQUILA_FDM_BAKE=bake.json cargo test -p tequila-format --test fdm_layer_lines -- --ignored
//!
//! Blender's sphere is built along its Z. Tequila's part is the same
//! sphere, once as it is, built along Z, and once Y up, built along Y,
//! which is what the group's turn into the print's frame undoes.

use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, Value};
use tequila_materials::groups::NodeGroup;
use tequila_materials::material_type::Param;
use tequila_materials::openpbr::Material;
use tequila_render_core::shade::{shade, Registers, TextureTable};
use tequila_texture::projection::{PointWorld, SurfacePoint};

/// Blender's point as Tequila's part has it.
fn blender_to_tequila(v: [f32; 3], along_z: bool) -> Vec3 {
    if along_z {
        vec3(v[0], v[1], v[2])
    } else {
        vec3(v[0], v[2], -v[1])
    }
}

fn read(json: &tequila_format::json::Value, key: &str) -> Vec<[f32; 3]> {
    json.get(key)
        .unwrap_or_else(|| panic!("no {key}"))
        .as_array()
        .iter()
        .map(|v| {
            let v = v.as_f32_array().expect("numbers");
            [v[0], v[1], v[2]]
        })
        .collect()
}

/// Output `output` of the group at every point.
fn tequila(output: u8, points: &[SurfacePoint], along_z: bool) -> Vec<Value> {
    let mut graph = MaterialGraph::from_material(Material::default());
    let group = graph.add_node(
        Node::new(NodeKind::Group(NodeGroup::FdmLayerLines)).with_literal(Socket::Input(0), Value::Float(if along_z { 1.0 } else { 0.0 })),
    );
    let surface = graph.output();
    graph.connect_output(group, output, surface, Socket::Param(Param::BaseColor));
    let program = graph.compile().expect("compiles");
    let register = program.driven.iter().find(|(p, _)| *p == Param::BaseColor).expect("driven").1;
    let textures = TextureTable::new();
    let (mut registers, mut scratch) = (Registers::new(), Material::default());
    points
        .iter()
        .map(|point| {
            shade(&program, &textures, point, &mut registers, &mut scratch);
            registers.value(register)
        })
        .collect()
}

#[test]
#[ignore = "needs a bake from scripts/fdm/bake_reference.py in TEQUILA_FDM_BAKE"]
fn fdm_layer_lines_match_blender() {
    for along_z in [true, false] {
        println!("built along {}", if along_z { "Z" } else { "Y" });
        compare(along_z);
    }
}

fn compare(along_z: bool) {
    let path = std::env::var("TEQUILA_FDM_BAKE").expect("TEQUILA_FDM_BAKE");
    let json = tequila_format::json::parse(&std::fs::read(path).unwrap()).unwrap();
    // Where Cycles baked each corner, in the part's own space.
    let offset = |by: f32| move |v: [f32; 3]| [v[0] - by, v[1] - by, v[2] - by];
    let positions: Vec<[f32; 3]> = read(&json, "object").into_iter().map(offset(0.02)).collect();
    let normals: Vec<[f32; 3]> = read(&json, "object_normal").into_iter().map(offset(1.0)).collect();
    let points: Vec<SurfacePoint> = positions
        .iter()
        .zip(&normals)
        .map(|(p, n)| {
            let (position, normal) = (blender_to_tequila(*p, along_z), blender_to_tequila(*n, along_z).normalize());
            SurfacePoint {
                position,
                normal,
                uv: vec2(0.0, 0.0),
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: Spectrum::ONE,
                world: PointWorld { position, normal, true_normal: normal, ..PointWorld::default() },
            }
        })
        .collect();
    let outputs = NodeGroup::FdmLayerLines.outputs();
    let index = |name: &str| outputs.iter().position(|o| o.name == name).unwrap() as u8;
    // Blender's channel, its offset, and Tequila's output: a number, or a
    // vector's component.
    type Check = (&'static str, usize, f32, &'static str, Option<usize>, f32);
    let checks: [Check; 12] = [
        ("height_cavity_layer", 0, 1.0, "Height (mm)", None, 2e-4),
        ("height_cavity_layer", 1, 0.0, "Cavity", None, 2e-3),
        ("height_cavity_layer", 2, 0.0, "Layer Random", None, 1e-4),
        ("wall_top_bottom", 0, 0.0, "Wall Factor", None, 1e-4),
        ("wall_top_bottom", 1, 0.0, "Top Mask", None, 1e-3),
        ("wall_top_bottom", 2, 0.0, "Bottom Mask", None, 1e-3),
        ("tangent", 0, 1.0, "Tangent", Some(0), 1e-3),
        ("tangent", 1, 1.0, "Tangent", Some(1), 1e-3),
        ("tangent", 2, 1.0, "Tangent", Some(2), 1e-3),
        ("position", 0, 100.0, "Position (mm)", Some(0), 2e-3),
        ("position", 1, 100.0, "Position (mm)", Some(1), 2e-3),
        ("position", 2, 100.0, "Position (mm)", Some(2), 2e-3),
    ];
    let mut failures = Vec::new();
    for (pass, channel, offset, output, component, tolerance) in checks {
        let blender = read(&json, pass);
        let ours = tequila(index(output), &points, along_z);
        let mut misses = 0;
        let mut worst = (0.0f32, 0usize);
        for (i, (b, t)) in blender.iter().zip(&ours).enumerate() {
            let expected = match component {
                // Blender's tangent is in its own frame; Tequila's in the
                // part's, Y up.
                Some(c) if output == "Tangent" => {
                    let v = blender_to_tequila([b[0] - offset, b[1] - offset, b[2] - offset], along_z);
                    [v.x, v.y, v.z][c]
                }
                _ => b[channel] - offset,
            };
            let got = match component {
                Some(c) => {
                    let v = t.as_vector();
                    [v.x, v.y, v.z][c]
                }
                None => t.as_float(),
            };
            let error = (got - expected).abs();
            if error > tolerance {
                misses += 1;
            }
            if error > worst.0 {
                worst = (error, i);
            }
        }
        // A floor or a comparison right on its edge can land either side
        // in single precision; a handful of such points is not a mismatch.
        let share = misses as f32 / blender.len() as f32;
        println!("{output}[{channel}]: {misses} of {} off by more than {tolerance} (worst {:.5} at {})", blender.len(), worst.0, worst.1);
        if share > 0.002 {
            failures.push(format!("{output}[{channel}]: {:.2}% off", share * 100.0));
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
